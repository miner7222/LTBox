//! Validate a complete set of partition images before programming any of them.

use super::{
    EdlError, EdlSession, QdlChan, Result, begin_partition_progress, padded_transfer_bytes, tr,
    update_flash_progress,
};
use std::fs::File;
use std::path::Path;

#[cfg(test)]
#[path = "partition_flash_tests.rs"]
mod tests;

/// One image and its target in the current device GPT.
pub struct PartitionFlash<'a> {
    pub label: &'a str,
    pub image: &'a Path,
    pub slot: u8,
    pub lun: u8,
}

/// Identifies the failed image without losing the underlying transport/I/O error.
#[derive(Debug, thiserror::Error)]
#[error("{partition}: {source}")]
pub struct PartitionFlashError {
    pub partition: String,
    #[source]
    pub source: EdlError,
}

struct PreparedFlash<'a> {
    request: &'a PartitionFlash<'a>,
    file: File,
    file_len: u64,
    start: u64,
    num_sectors: usize,
}

impl<'a> PreparedFlash<'a> {
    fn open(
        request: &'a PartitionFlash<'a>,
        start: u64,
        end: u64,
        sector_size: usize,
    ) -> Result<Self> {
        let label = request.label;
        let span = EdlSession::partition_span_sectors(label, start, end)?;
        let file = File::open(request.image)?;
        let metadata = file.metadata()?;
        let file_len = metadata.len();
        if !metadata.is_file() || file_len == 0 {
            return Err(EdlError::Session(format!(
                "Flash {label}: image must be a nonempty regular file"
            )));
        }
        if sector_size == 0 {
            return Err(EdlError::Session("Storage sector size is zero".into()));
        }
        let image_sectors = file_len.div_ceil(sector_size as u64);
        if image_sectors > span as u64 {
            return Err(EdlError::Session(format!(
                "Flash {label}: image is {image_sectors} sectors but the partition spans only {span}"
            )));
        }
        let num_sectors = usize::try_from(image_sectors).map_err(|_| {
            EdlError::Session(format!("Flash {label}: image sector count exceeds usize"))
        })?;
        // qdl computes the padded transfer length in usize.
        num_sectors.checked_mul(sector_size).ok_or_else(|| {
            EdlError::Session(format!("Flash {label}: padded image size exceeds usize"))
        })?;
        Ok(Self {
            request,
            file,
            file_len,
            start,
            num_sectors,
        })
    }
}

impl EdlSession {
    /// Resolve every target from the current GPT and open/check every image
    /// before issuing the first program command. Open handles are retained for
    /// the writes, so a later pathname replacement cannot substitute an image.
    /// Call after any rawprogram/GPT updates; source image size is not capacity.
    ///
    /// `on_partition` runs before each program operation, after the entire batch
    /// passes preflight. `on_write_start` runs at the transport's first command
    /// write attempt for each partition, even if that attempt fails. It does not
    /// imply successful programming. A failure stops the remaining writes.
    pub fn flash_partition_batch(
        &mut self,
        partitions: &[PartitionFlash<'_>],
        log: &mut Vec<String>,
        mut on_partition: impl FnMut(&str, &Path, &mut Vec<String>),
        mut on_write_start: impl FnMut(),
    ) -> std::result::Result<(), PartitionFlashError> {
        let mut prepared = Vec::with_capacity(partitions.len());
        for request in partitions {
            let result = (|| {
                ltbox_core::live!(
                    log,
                    "[EDL] {} '{}' on LUN {}...",
                    tr("log_edl_lookup_partition"),
                    request.label,
                    request.lun
                );
                let config = self.dev.fh_config();
                if config.storage_sector_size == 0
                    || config.send_buffer_size < config.storage_sector_size
                {
                    return Err(EdlError::Session(
                        "Invalid storage sector size or send buffer size".into(),
                    ));
                }
                let sector_size = config.storage_sector_size;
                let (start, end) = self.find_partition(request.label, request.slot, request.lun)?;
                PreparedFlash::open(request, start, end, sector_size)
            })();
            prepared.push(result.map_err(|source| PartitionFlashError {
                partition: request.label.to_string(),
                source,
            })?);
        }

        // The whole batch is known after preflight, so publish its complete
        // denominator before the first write. This keeps cumulative progress
        // stable across the generated overlay images in a full flash.
        let sector_size = self.dev.fh_config().storage_sector_size;
        for image in &prepared {
            super::register_flash_bytes(
                padded_transfer_bytes(image.num_sectors, sector_size).map_err(|source| {
                    PartitionFlashError {
                        partition: image.request.label.to_string(),
                        source,
                    }
                })?,
            );
        }

        for mut image in prepared {
            let request = image.request;
            let transfer_bytes =
                padded_transfer_bytes(image.num_sectors, sector_size).map_err(|source| {
                    PartitionFlashError {
                        partition: request.label.to_string(),
                        source,
                    }
                })?;
            begin_partition_progress(request.label, transfer_bytes, false);
            let mut last_percent = None;
            on_partition(request.label, request.image, log);
            ltbox_core::live!(
                log,
                "[EDL] {} {} ← {} ({} bytes, {} sectors)",
                tr("log_edl_flash_cmd"),
                request.label,
                request.image.display(),
                image.file_len,
                image.num_sectors
            );
            qdl::firehose_program_storage_with_callbacks(
                &mut self.dev,
                &mut image.file,
                request.label,
                image.num_sectors,
                request.slot,
                request.lun,
                &image.start.to_string(),
                |completed, total| {
                    update_flash_progress(&mut last_percent, request.label, completed, total)
                },
                &mut on_write_start,
            )
            .map_err(|error| PartitionFlashError {
                partition: request.label.to_string(),
                source: EdlError::Session(format!("Partition write failed: {error}")),
            })?;
            ltbox_core::live!(log, "[EDL] {} {}", tr("log_edl_flashed"), request.label);
        }
        Ok(())
    }
}
