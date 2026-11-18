//! Validate a complete set of partition images before programming any of them.

use super::{
    EdlError, EdlSession, QdlChan, Result, begin_partition_progress, padded_transfer_bytes,
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

/// A GPT-resolved write or erase. `None` erases the whole partition.
pub struct PartitionOperation<'a> {
    pub label: &'a str,
    pub image: Option<&'a Path>,
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
    request: &'a PartitionOperation<'a>,
    file: Option<File>,
    file_len: u64,
    start: u64,
    num_sectors: usize,
}

impl<'a> PreparedFlash<'a> {
    fn open(
        request: &'a PartitionOperation<'a>,
        start: u64,
        end: u64,
        sector_size: usize,
    ) -> Result<Self> {
        let label = request.label;
        let span = EdlSession::partition_span_sectors(label, start, end)?;
        let Some(path) = request.image else {
            return Ok(Self {
                request,
                file: None,
                file_len: 0,
                start,
                num_sectors: span,
            });
        };
        let file = File::open(path)?;
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
            file: Some(file),
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
    /// `on_partition` runs before each program operation, after the whole batch
    /// passes preflight. `on_write_start` runs at each partition's first command
    /// write attempt, even a failed one; it does not imply success. A failure
    /// stops the remaining writes.
    pub fn flash_partition_batch(
        &mut self,
        partitions: &[PartitionFlash<'_>],
        log: &mut Vec<String>,
        mut on_partition: impl FnMut(&str, &Path, &mut Vec<String>),
        mut on_write_start: impl FnMut(),
    ) -> std::result::Result<(), PartitionFlashError> {
        let operations: Vec<_> = partitions
            .iter()
            .map(|request| PartitionOperation {
                label: request.label,
                image: Some(request.image),
                slot: request.slot,
                lun: request.lun,
            })
            .collect();
        self.apply_partition_batch(
            &operations,
            log,
            |request, log| {
                if let Some(path) = request.image {
                    on_partition(request.label, path, log);
                }
            },
            &mut on_write_start,
        )
    }

    /// Validate and retain all sources before any write or erase, preserving
    /// requested order. A preflight failure performs no destructive command.
    /// Erase callbacks conservatively mark the attempt immediately before erase.
    pub fn apply_partition_batch(
        &mut self,
        partitions: &[PartitionOperation<'_>],
        log: &mut Vec<String>,
        mut on_partition: impl FnMut(&PartitionOperation<'_>, &mut Vec<String>),
        mut on_write_start: impl FnMut(),
    ) -> std::result::Result<(), PartitionFlashError> {
        let mut prepared = Vec::with_capacity(partitions.len());
        let mut seen = std::collections::BTreeSet::new();
        for request in partitions {
            let result = (|| {
                if !seen.insert((request.lun, request.slot, request.label)) {
                    return Err(EdlError::Session(format!(
                        "Duplicate partition operation: {}",
                        request.label
                    )));
                }
                ltbox_core::live_debug!(
                    log,
                    "[EDL] {}",
                    ltbox_core::tr_args!(
                        "log_edl_lookup_partition",
                        part = request.label,
                        lun = request.lun
                    )
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

        // Register the whole batch before the first write so cumulative
        // progress stays stable across a full flash's overlay images.
        let sector_size = self.dev.fh_config().storage_sector_size;
        for image in &prepared {
            if image.file.is_none() {
                continue;
            }
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
            on_partition(request, log);
            let Some(file) = image.file.as_mut() else {
                on_write_start();
                self.erase_partition_at(
                    request.label,
                    request.lun,
                    &image.start.to_string(),
                    image.num_sectors,
                    log,
                )
                .map_err(|source| PartitionFlashError {
                    partition: request.label.into(),
                    source,
                })?;
                continue;
            };
            let transfer_bytes =
                padded_transfer_bytes(image.num_sectors, sector_size).map_err(|source| {
                    PartitionFlashError {
                        partition: request.label.to_string(),
                        source,
                    }
                })?;
            begin_partition_progress(request.label, transfer_bytes, false);
            let mut last_percent = None;
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!(
                    "log_edl_flash_image",
                    part = request.label,
                    path = request
                        .image
                        .expect("prepared write has an image")
                        .display(),
                    size = ltbox_core::log_format::bytes(image.file_len),
                    sectors = image.num_sectors,
                    lun = request.lun
                )
            );
            qdl::firehose_program_storage_with_callbacks(
                &mut self.dev,
                file,
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
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_flashed", part = request.label)
            );
        }
        Ok(())
    }
}
