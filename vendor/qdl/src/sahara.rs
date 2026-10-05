// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
use anstream::println;
use std::{
    cmp::min,
    ffi::CStr,
    fs::File,
    io::{Read, Write},
    mem::{self, size_of_val},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::types::{QdlBackend, QdlChan};
use crate::wire::{Reader, Wire};

const SAHARA_STATUS_SUCCESS: u32 = 0;

#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum SaharaMode {
    WaitingForImage = 0x0,
    MemoryDebug = 0x2,
    Command = 0x3,
}

impl SaharaMode {
    fn from_u32(value: u32) -> Option<Self> {
        match value {
            0x0 => Some(Self::WaitingForImage),
            0x2 => Some(Self::MemoryDebug),
            0x3 => Some(Self::Command),
            _ => None,
        }
    }
}

impl Wire for SaharaMode {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend((*self as u32).to_le_bytes());
    }

    fn read_le(reader: &mut Reader<'_>) -> Result<Self> {
        let value = reader.u32()?;
        Self::from_u32(value).ok_or_else(|| anyhow!("unknown Sahara mode {value}"))
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u32)]
pub enum SaharaCmdModeCmd {
    Nop = 0x0,
    ReadSerialNum = 0x1,
    ReadHwId = 0x2,
    ReadOemKeyHash = 0x3,
}

impl SaharaCmdModeCmd {
    fn from_u32(value: u32) -> Option<Self> {
        match value {
            0x0 => Some(Self::Nop),
            0x1 => Some(Self::ReadSerialNum),
            0x2 => Some(Self::ReadHwId),
            0x3 => Some(Self::ReadOemKeyHash),
            _ => None,
        }
    }
}

impl Wire for SaharaCmdModeCmd {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend((*self as u32).to_le_bytes());
    }

    fn read_le(reader: &mut Reader<'_>) -> Result<Self> {
        let value = reader.u32()?;
        Self::from_u32(value).ok_or_else(|| anyhow!("unknown Sahara command-mode command {value}"))
    }
}

// Encoded as the bare u32 value (not an entry index).
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum SaharaCmd {
    SaharaHello = 0x1,      /* Device sends HELLO at init */
    SaharaHelloResp = 0x2,  /* Host responds with a version number */
    SaharaReadData = 0x3,   /* Device requests an image to read */
    SaharaEndOfImage = 0x4, /* Device signals EOF */
    SaharaDone = 0x5,       /* Host reassures the Device EOF was understood */
    SaharaDoneResp = 0x6,   /* Device requires more images if status == 0 */
    SaharaReset = 0x7,      /* Host asks Device to stop the current process */
    SaharaResetResp = 0x8,  /* Device acks the reset */
    /* Proto >= 2.0 */
    SaharaMemDebug = 0x9,
    SaharaMemRead = 0xa,
    /* Proto >= 2.1 */
    SaharaCommandReady = 0xb,
    SaharaSwitchMode = 0xc,
    SaharaExecute = 0xd,
    SaharaExecuteResp = 0xe,
    SaharaExecuteData = 0xf,
    /* Proto >= 2.5 */
    SaharaMemDebug64 = 0x10,
    SaharaMemRead64 = 0x11,
    /* Proto >= 2.8 */
    SaharaReadData64 = 0x12,
    /* Proto >= 2.9 */
    SaharaResetState = 0x13,
    /* Proto >= 3.0 */
    SaharaWriteData = 0x14,

    /* This isn't part of spec, but rather "<?xm" (XML) suggesting Sahara mode is over */
    SaharaXML = 0x6d783f3c,
}

impl SaharaCmd {
    fn from_u32(value: u32) -> Option<Self> {
        Some(match value {
            0x1 => Self::SaharaHello,
            0x2 => Self::SaharaHelloResp,
            0x3 => Self::SaharaReadData,
            0x4 => Self::SaharaEndOfImage,
            0x5 => Self::SaharaDone,
            0x6 => Self::SaharaDoneResp,
            0x7 => Self::SaharaReset,
            0x8 => Self::SaharaResetResp,
            0x9 => Self::SaharaMemDebug,
            0xa => Self::SaharaMemRead,
            0xb => Self::SaharaCommandReady,
            0xc => Self::SaharaSwitchMode,
            0xd => Self::SaharaExecute,
            0xe => Self::SaharaExecuteResp,
            0xf => Self::SaharaExecuteData,
            0x10 => Self::SaharaMemDebug64,
            0x11 => Self::SaharaMemRead64,
            0x12 => Self::SaharaReadData64,
            0x13 => Self::SaharaResetState,
            0x14 => Self::SaharaWriteData,
            0x6d783f3c => Self::SaharaXML,
            _ => return None,
        })
    }
}

impl Wire for SaharaCmd {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend((*self as u32).to_le_bytes());
    }

    fn read_le(reader: &mut Reader<'_>) -> Result<Self> {
        let value = reader.u32()?;
        Self::from_u32(value).ok_or_else(|| anyhow!("unknown Sahara command {value}"))
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct HelloReq {
    pub ver: u32,
    pub compatible: u32,
    pub max_len: u32,
    pub mode: SaharaMode,
    unk0: u32,
    unk1: u32,
    unk2: u32,
    unk3: u32,
    unk4: u32,
    unk5: u32,
}

impl Wire for HelloReq {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.ver.to_le_bytes());
        out.extend(self.compatible.to_le_bytes());
        out.extend(self.max_len.to_le_bytes());
        self.mode.write_le(out);
        for v in [
            self.unk0, self.unk1, self.unk2, self.unk3, self.unk4, self.unk5,
        ] {
            out.extend(v.to_le_bytes());
        }
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            ver: r.u32()?,
            compatible: r.u32()?,
            max_len: r.u32()?,
            mode: SaharaMode::read_le(r)?,
            unk0: r.u32()?,
            unk1: r.u32()?,
            unk2: r.u32()?,
            unk3: r.u32()?,
            unk4: r.u32()?,
            unk5: r.u32()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct HelloResp {
    ver: u32,
    compatible: u32,
    status: u32,
    mode: SaharaMode,
    unk0: u32,
    unk1: u32,
    unk2: u32,
    unk3: u32,
    unk4: u32,
    unk5: u32,
}

impl Wire for HelloResp {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.ver.to_le_bytes());
        out.extend(self.compatible.to_le_bytes());
        out.extend(self.status.to_le_bytes());
        self.mode.write_le(out);
        for v in [
            self.unk0, self.unk1, self.unk2, self.unk3, self.unk4, self.unk5,
        ] {
            out.extend(v.to_le_bytes());
        }
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            ver: r.u32()?,
            compatible: r.u32()?,
            status: r.u32()?,
            mode: SaharaMode::read_le(r)?,
            unk0: r.u32()?,
            unk1: r.u32()?,
            unk2: r.u32()?,
            unk3: r.u32()?,
            unk4: r.u32()?,
            unk5: r.u32()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ReadReq {
    pub image: u32,
    pub offset: u32,
    pub len: u32,
}

impl Wire for ReadReq {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.image.to_le_bytes());
        out.extend(self.offset.to_le_bytes());
        out.extend(self.len.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            image: r.u32()?,
            offset: r.u32()?,
            len: r.u32()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Eoi {
    pub image: u32,
    pub status: u32,
}

impl Wire for Eoi {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.image.to_le_bytes());
        out.extend(self.status.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            image: r.u32()?,
            status: r.u32()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DoneReq {}

impl Wire for DoneReq {
    fn write_le(&self, _out: &mut Vec<u8>) {}

    fn read_le(_r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {})
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DoneResp {
    pub status: u32,
}

impl Wire for DoneResp {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.status.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { status: r.u32()? })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ResetReq {}

impl Wire for ResetReq {
    fn write_le(&self, _out: &mut Vec<u8>) {}

    fn read_le(_r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {})
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ResetResp {}

impl Wire for ResetResp {
    fn write_le(&self, _out: &mut Vec<u8>) {}

    fn read_le(_r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {})
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CommandReady {}

impl Wire for CommandReady {
    fn write_le(&self, _out: &mut Vec<u8>) {}

    fn read_le(_r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {})
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct SwitchMode {
    mode: SaharaMode,
}

impl Wire for SwitchMode {
    fn write_le(&self, out: &mut Vec<u8>) {
        self.mode.write_le(out);
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            mode: SaharaMode::read_le(r)?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ExecResp {
    pub command: SaharaCmdModeCmd,
    pub len: u32,
}

impl Wire for ExecResp {
    fn write_le(&self, out: &mut Vec<u8>) {
        self.command.write_le(out);
        out.extend(self.len.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            command: SaharaCmdModeCmd::read_le(r)?,
            len: r.u32()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Debug64Req {
    addr: u64,
    len: u64,
}

impl Wire for Debug64Req {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.addr.to_le_bytes());
        out.extend(self.len.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            addr: r.u64()?,
            len: r.u64()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ReadMem64Req {
    addr: u64,
    len: u64,
}

impl Wire for ReadMem64Req {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.addr.to_le_bytes());
        out.extend(self.len.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            addr: r.u64()?,
            len: r.u64()?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ReadData64Req {
    pub image: u64,
    pub offset: u64,
    pub len: u64,
}

impl Wire for ReadData64Req {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.image.to_le_bytes());
        out.extend(self.offset.to_le_bytes());
        out.extend(self.len.to_le_bytes());
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            image: r.u64()?,
            offset: r.u64()?,
            len: r.u64()?,
        })
    }
}

/// Encoded as the bare inner struct (no variant tag). The variant is chosen by
/// the packet's command, so decoding is done per command in
/// `sahara_parse_packet`.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub enum SaharaPacketBody {
    // Packets accepted in WaitingForImage mode
    HelloReq(HelloReq),
    HelloResp(HelloResp),
    ReadReq(ReadReq),
    Eoi(Eoi),
    DoneReq(DoneReq),
    DoneResp(DoneResp),
    ResetReq(ResetReq),
    ResetResp(ResetResp),
    CommandReady(CommandReady),
    SwitchMode(SwitchMode),
    ExecResp(ExecResp),
    Debug64Req(Debug64Req),
    ReadMem64Req(ReadMem64Req),
    ReadData64Req(ReadData64Req),

    // Packets accepted in Command mode
    Command(SaharaCmdModeCmd),
}

impl SaharaPacketBody {
    fn write_le(&self, out: &mut Vec<u8>) {
        match self {
            Self::HelloReq(b) => b.write_le(out),
            Self::HelloResp(b) => b.write_le(out),
            Self::ReadReq(b) => b.write_le(out),
            Self::Eoi(b) => b.write_le(out),
            Self::DoneReq(b) => b.write_le(out),
            Self::DoneResp(b) => b.write_le(out),
            Self::ResetReq(b) => b.write_le(out),
            Self::ResetResp(b) => b.write_le(out),
            Self::CommandReady(b) => b.write_le(out),
            Self::SwitchMode(b) => b.write_le(out),
            Self::ExecResp(b) => b.write_le(out),
            Self::Debug64Req(b) => b.write_le(out),
            Self::ReadMem64Req(b) => b.write_le(out),
            Self::ReadData64Req(b) => b.write_le(out),
            Self::Command(b) => b.write_le(out),
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct SaharaPacket {
    pub cmd: SaharaCmd,
    pub len: u32,
    pub body: SaharaPacketBody,
}

impl SaharaPacket {
    fn to_le_bytes(self) -> Vec<u8> {
        let mut out = Vec::new();
        self.cmd.write_le(&mut out);
        out.extend(self.len.to_le_bytes());
        self.body.write_le(&mut out);
        out
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct RamdumpTable64 {
    save_pref: u64,
    base: u64,
    len: u64,
    description: [u8; 20],
    filename: [u8; 20],
}

impl Wire for RamdumpTable64 {
    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend(self.save_pref.to_le_bytes());
        out.extend(self.base.to_le_bytes());
        out.extend(self.len.to_le_bytes());
        out.extend(self.description);
        out.extend(self.filename);
    }

    fn read_le(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            save_pref: r.u64()?,
            base: r.u64()?,
            len: r.u64()?,
            description: r.array::<20>()?,
            filename: r.array::<20>()?,
        })
    }
}

pub fn sahara_send_img_to_device<T: Read + Write>(
    channel: &mut T,
    img_arr: &mut [Option<Vec<u8>>],
    image_idx: u64,
    image_offset: u64,
    image_len: u64,
) -> Result<usize, anyhow::Error> {
    let image = if img_arr.len() == 1 { 0 } else { image_idx };
    let Some(Some(buf)) = img_arr.get(image as usize) else {
        bail!("Sahara requested missing image ID {}", image_idx,);
    };
    if (image_offset + image_len) as usize > buf.len() {
        bail!(
            "Attempted OOB read {} > {}",
            image_offset + image_len,
            buf.len()
        );
    }

    channel
        .write_all(&buf[image_offset as usize..(image_offset + image_len) as usize])
        .map(|_| image_len as usize)
        .map_err(|e| e.into())
}

fn sahara_send_generic<T: Read + Write>(
    channel: &mut T,
    cmd: SaharaCmd,
    body: SaharaPacketBody,
    body_len: usize,
) -> Result<usize> {
    let pkt = SaharaPacket {
        cmd,
        len: (size_of_val(&cmd) + size_of::<u32>() + body_len) as u32,
        body,
    };

    let serialized = pkt.to_le_bytes();
    let len = serialized.len();
    channel
        .write_all(&serialized)
        .map(|_| len)
        .map_err(|e| e.into())
}

const SAHARA_VERSION: u32 = 2;
pub fn sahara_send_hello_rsp<T: Read + Write>(channel: &mut T, mode: SaharaMode) -> Result<usize> {
    let data = HelloResp {
        ver: SAHARA_VERSION,
        compatible: 1,
        status: SAHARA_STATUS_SUCCESS,
        mode,
        unk0: 0,
        unk1: 0,
        unk2: 0,
        unk3: 0,
        unk4: 0,
        unk5: 0,
    };

    sahara_send_generic(
        channel,
        SaharaCmd::SaharaHelloResp,
        SaharaPacketBody::HelloResp(data),
        size_of_val(&data),
    )
}

pub fn sahara_send_done<T: Read + Write>(channel: &mut T) -> Result<usize> {
    let data = DoneReq {};

    sahara_send_generic(
        channel,
        SaharaCmd::SaharaDone,
        SaharaPacketBody::DoneReq(data),
        size_of_val(&data),
    )
}

pub fn sahara_send_cmd_exec<T: Read + Write>(
    channel: &mut T,
    command: SaharaCmdModeCmd,
) -> Result<usize, anyhow::Error> {
    sahara_send_generic(
        channel,
        SaharaCmd::SaharaExecute,
        SaharaPacketBody::Command(command),
        size_of_val(&command),
    )
}

pub fn sahara_send_cmd_data<T: Read + Write>(
    channel: &mut T,
    command: SaharaCmdModeCmd,
) -> Result<usize, anyhow::Error> {
    sahara_send_generic(
        channel,
        SaharaCmd::SaharaExecuteData,
        SaharaPacketBody::Command(command),
        size_of_val(&command),
    )
}

pub fn sahara_reset<T: Read + Write>(channel: &mut T) -> Result<usize, anyhow::Error> {
    let data = ResetReq {};

    sahara_send_generic(
        channel,
        SaharaCmd::SaharaReset,
        SaharaPacketBody::ResetReq(data),
        size_of_val(&data),
    )
}

pub fn sahara_switch_mode<T: Read + Write>(
    channel: &mut T,
    mode: SaharaMode,
) -> Result<usize, anyhow::Error> {
    let data = SwitchMode { mode };

    sahara_send_generic(
        channel,
        SaharaCmd::SaharaSwitchMode,
        SaharaPacketBody::SwitchMode(data),
        size_of_val(&data),
    )
}

pub fn sahara_get_ramdump_tbl<T: Read + Write>(
    channel: &mut T,
    addr: u64,
    len: u64,
    verbose: bool,
) -> Result<Vec<RamdumpTable64>, anyhow::Error> {
    let data = ReadMem64Req { addr, len };

    sahara_send_generic(
        channel,
        SaharaCmd::SaharaMemRead64,
        SaharaPacketBody::ReadMem64Req(data),
        size_of_val(&data),
    )?;

    let entry_size = size_of::<RamdumpTable64>();
    let num_chunks = len as usize / entry_size;
    let mut tbl = Vec::<RamdumpTable64>::with_capacity(num_chunks);

    let mut buf = vec![0u8; len as usize];
    channel.read_exact(&mut buf)?;

    if verbose {
        println!("Available images:");
    }
    for i in 0..num_chunks {
        let entry = RamdumpTable64::from_le_bytes(&buf[i * entry_size..])?;
        tbl.push(entry);
        if verbose {
            println!(
                "\t{} (0x{:x} @ 0x{:x}){}",
                String::from_utf8(entry.filename.to_vec())?,
                entry.len,
                entry.base,
                match entry.save_pref {
                    0 => "",
                    _ => " *",
                }
            );
        }
    }

    Ok(tbl)
}

fn sahara_dump_region<T: QdlChan>(
    channel: &mut T,
    entry: RamdumpTable64,
    output: &mut impl Write,
) -> Result<()> {
    let mut progress = crate::operation_log::Transfer::new(
        false,
        format!("Sahara {}", String::from_utf8(entry.filename.to_vec())?),
        entry.len,
    );

    let mut bytes_read = 0usize;
    while bytes_read < entry.len as usize {
        let chunk_size = min(4096, entry.len as usize - bytes_read);
        let mut buf = vec![0u8; chunk_size];
        let data = ReadMem64Req {
            addr: entry.base + bytes_read as u64,
            len: chunk_size as u64,
        };

        sahara_send_generic(
            channel,
            SaharaCmd::SaharaMemRead64,
            SaharaPacketBody::ReadMem64Req(data),
            size_of_val(&data),
        )?;
        channel.flush()?;

        let n = channel.read(&mut buf)?;
        bytes_read += n;

        // Issue a dummy read to consume the ZLP
        if channel.fh_config().backend == QdlBackend::Usb && n.is_multiple_of(512) {
            let _ = channel.read(&mut []);
        }

        output.write_all(&buf[..n])?;
        progress.add(n as u64);
    }
    drop(progress);

    Ok(())
}

pub fn sahara_dump_regions<T: QdlChan>(
    channel: &mut T,
    dump_tbl: Vec<RamdumpTable64>,
    regions_to_dump: Vec<String>,
) -> Result<()> {
    // Make all of them lowercase for better UX
    let regions_to_dump = regions_to_dump
        .iter()
        .map(|rname| rname.to_ascii_lowercase())
        .collect::<Vec<String>>();

    std::fs::create_dir_all("ramdump/")?;
    let filtered_list: Vec<RamdumpTable64> = match regions_to_dump.len() {
        // Dump everything with save_pref == true if no argument was provided
        0 => dump_tbl
            .iter()
            .filter(|e| e.save_pref != 0)
            .copied()
            .collect(),
        _ => dump_tbl
            .iter()
            .filter(|dump_entry| {
                regions_to_dump.contains(
                    &String::from_utf8(dump_entry.filename.to_vec())
                        .unwrap_or("".to_owned())
                        .to_ascii_lowercase()
                        .split('.') // Ignore file extensions proposed by ramdump
                        .next()
                        .unwrap_or("")
                        .to_owned(),
                )
            })
            .copied()
            .collect(),
    };
    for entry in filtered_list {
        let fname = CStr::from_bytes_until_nul(&entry.filename)
            .context("Ramdump table entry filename is not NUL-terminated")?
            .to_str()?
            .to_owned();

        let mut f = File::create(std::path::Path::new(&format!("ramdump/{fname}")))
            .with_context(|| format!("Couldn't create ramdump output file for {fname}"))?;
        sahara_dump_region(channel, entry, &mut f)?;
    }

    Ok(())
}

fn sahara_read_packet<T: Read>(channel: &mut T, verbose: bool) -> Result<SaharaPacket> {
    const HEADER_LEN: usize = size_of::<SaharaCmd>() + size_of::<u32>();
    const MAX_PACKET_LEN: usize = 4096;

    let mut header = [0u8; HEADER_LEN];
    channel.read_exact(&mut header)?;

    let packet_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    if !(HEADER_LEN..=MAX_PACKET_LEN).contains(&packet_len) {
        bail!("Invalid Sahara packet length {packet_len}");
    }

    let mut packet = vec![0u8; packet_len];
    packet[..HEADER_LEN].copy_from_slice(&header);
    channel.read_exact(&mut packet[HEADER_LEN..])?;

    sahara_parse_packet(&packet, verbose)
}

pub fn sahara_run<T: QdlChan>(
    channel: &mut T,
    sahara_mode: SaharaMode,
    sahara_command: Option<SaharaCmdModeCmd>,
    images: &mut [Option<Vec<u8>>],
    filenames: Vec<String>,
    verbose: bool,
) -> Result<Vec<u8>> {
    loop {
        let pkt = sahara_read_packet(channel, verbose)?;
        let pktsize = size_of_val(&pkt.cmd) + size_of_val(&pkt.len);

        match pkt.cmd {
            SaharaCmd::SaharaHello => {
                if let SaharaPacketBody::HelloReq(req) = pkt.body {
                    assert_eq!(pkt.len as usize, pktsize + mem::size_of::<HelloReq>());

                    // MemoryDebug mode can only be entered if the device offers it
                    let mode = if sahara_mode == SaharaMode::MemoryDebug
                        && req.mode == SaharaMode::MemoryDebug
                    {
                        SaharaMode::MemoryDebug
                    } else {
                        sahara_mode
                    };
                    sahara_send_hello_rsp(channel, mode)?;
                }
            }
            SaharaCmd::SaharaReadData => {
                if let SaharaPacketBody::ReadReq(rr) = pkt.body {
                    assert_eq!(pkt.len as usize, pktsize + mem::size_of::<ReadReq>());
                    sahara_send_img_to_device(
                        channel,
                        images,
                        rr.image as u64,
                        rr.offset as u64,
                        rr.len as u64,
                    )?;
                }
            }
            SaharaCmd::SaharaEndOfImage => {
                if let SaharaPacketBody::Eoi(req) = pkt.body {
                    assert_eq!(pkt.len as usize, pktsize + mem::size_of::<Eoi>());

                    if req.status == 0 {
                        sahara_send_done(channel)?;
                    } else {
                        bail!("Received unsuccessful End of Image packet");
                    }
                }
            }
            SaharaCmd::SaharaDoneResp => {
                if let SaharaPacketBody::DoneResp(req) = pkt.body
                    && (req.status == 1 /* COMPLETE */ /* 8916 bug */ ||
                     images.len() == 1)
                {
                    crate::operation_log::diagnostic("Sahara programmer transfer complete");
                    return Ok(vec![]);
                }
            }
            SaharaCmd::SaharaCommandReady => {
                assert_eq!(pkt.len as usize, pktsize);
                match sahara_command {
                    Some(cmd) => sahara_send_cmd_exec(channel, cmd),
                    None => bail!("Missing sahara command"),
                }?;
            }
            SaharaCmd::SaharaExecuteResp => {
                if let SaharaPacketBody::ExecResp(resp) = pkt.body {
                    let mut resp_buf = vec![0u8; resp.len as usize];

                    // Indicate we're ready to receive the requested amount of data
                    sahara_send_cmd_data(channel, resp.command)?;

                    channel.read_exact(&mut resp_buf)?;

                    // Got everything we want, exit command mode
                    sahara_switch_mode(channel, SaharaMode::WaitingForImage)?;

                    return Ok(resp_buf);
                }
            }
            SaharaCmd::SaharaMemDebug64 => {
                if let SaharaPacketBody::Debug64Req(req) = pkt.body {
                    assert_eq!(pkt.len as usize, pktsize + mem::size_of::<Debug64Req>());

                    // Receive the dump info table
                    let dump_tbl = sahara_get_ramdump_tbl(channel, req.addr, req.len, verbose)?;

                    // Grab some (possibly all) of the available regions
                    sahara_dump_regions(channel, dump_tbl, filenames)?;

                    return Ok(vec![]);
                }
            }
            SaharaCmd::SaharaReadData64 => {
                if let SaharaPacketBody::ReadData64Req(rr) = pkt.body {
                    assert_eq!(pkt.len as usize, pktsize + mem::size_of::<ReadData64Req>());
                    sahara_send_img_to_device(channel, images, rr.image, rr.offset, rr.len)?;
                }
            }
            SaharaCmd::SaharaResetResp => {
                assert_eq!(pkt.len as usize, pktsize);
                return Ok(vec![]);
            }
            SaharaCmd::SaharaXML => {
                // Todo: make this optionally "fine"
                println!("Device booted into the loader already");
                return Ok(vec![]);
            }
            _ => bail!("Got unexpected packet {:?}", pkt),
        }
    }
}

fn sahara_parse_packet(buf: &[u8], verbose: bool) -> Result<SaharaPacket> {
    let (cmd, rest) = buf
        .split_first_chunk::<4>()
        .ok_or_else(|| anyhow!("Malformed packet, too short: {buf:?}"))?;
    let (len, args) = rest
        .split_first_chunk::<4>()
        .ok_or_else(|| anyhow!("Malformed packet, too short: {buf:?}"))?;

    let cmd = SaharaCmd::from_u32(u32::from_le_bytes(*cmd))
        .ok_or_else(|| anyhow!("Got unknown command {}", u32::from_le_bytes(*cmd)))?;

    let ret = SaharaPacket {
        cmd,
        len: u32::from_le_bytes(*len),
        body: match cmd {
            SaharaCmd::SaharaHello => SaharaPacketBody::HelloReq(
                HelloReq::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaHelloResp => SaharaPacketBody::HelloResp(
                HelloResp::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaReadData => SaharaPacketBody::ReadReq(
                ReadReq::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaEndOfImage => SaharaPacketBody::Eoi(
                Eoi::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaDone => SaharaPacketBody::DoneReq(DoneReq {}),
            SaharaCmd::SaharaDoneResp => SaharaPacketBody::DoneResp(
                DoneResp::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaResetResp => SaharaPacketBody::ResetResp(ResetResp {}),
            SaharaCmd::SaharaCommandReady => SaharaPacketBody::CommandReady(CommandReady {}),
            SaharaCmd::SaharaExecuteResp => SaharaPacketBody::ExecResp(
                ExecResp::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaExecuteData => SaharaPacketBody::Command(
                SaharaCmdModeCmd::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaMemDebug64 => SaharaPacketBody::Debug64Req(
                Debug64Req::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaMemRead64 => SaharaPacketBody::ReadMem64Req(
                ReadMem64Req::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaReadData64 => SaharaPacketBody::ReadData64Req(
                ReadData64Req::from_le_bytes(args)
                    .with_context(|| format!("Malformed {cmd:?} packet body"))?,
            ),
            SaharaCmd::SaharaXML => bail!(
                "Got Firehose command while expecting Sahara command: {:?}",
                String::from_utf8_lossy(buf)
            ),
            _ => bail!("Got unimplemented command: {:?}", buf),
        },
    };

    if verbose {
        println!("{:?}", ret);
    }

    Ok(ret)
}

#[cfg(test)]
mod tests {
    use super::{DoneResp, Eoi, SaharaCmd, SaharaPacket, SaharaPacketBody, sahara_read_packet};
    use std::io::{BufReader, Cursor, ErrorKind};

    fn eoi_packet() -> Vec<u8> {
        SaharaPacket {
            cmd: SaharaCmd::SaharaEndOfImage,
            len: 16,
            body: SaharaPacketBody::Eoi(Eoi {
                image: 13,
                status: 0,
            }),
        }
        .to_le_bytes()
    }

    fn done_response_packet() -> Vec<u8> {
        SaharaPacket {
            cmd: SaharaCmd::SaharaDoneResp,
            len: 12,
            body: SaharaPacketBody::DoneResp(DoneResp { status: 1 }),
        }
        .to_le_bytes()
    }

    #[test]
    fn reads_packet_split_across_transport_reads() {
        let packet = eoi_packet();
        let mut reader = BufReader::with_capacity(5, Cursor::new(packet));

        let parsed = sahara_read_packet(&mut reader, false).unwrap();

        assert_eq!(parsed.cmd, SaharaCmd::SaharaEndOfImage);
        assert_eq!(parsed.len, 16);
        let SaharaPacketBody::Eoi(eoi) = parsed.body else {
            panic!("expected EndOfImage body");
        };
        assert_eq!(eoi.image, 13);
        assert_eq!(eoi.status, 0);
    }

    #[test]
    fn leaves_coalesced_packet_for_next_read() {
        let mut transport_data = eoi_packet();
        transport_data.extend(done_response_packet());
        let mut reader = Cursor::new(transport_data);

        let first = sahara_read_packet(&mut reader, false).unwrap();
        let second = sahara_read_packet(&mut reader, false).unwrap();

        assert_eq!(first.cmd, SaharaCmd::SaharaEndOfImage);
        assert_eq!(second.cmd, SaharaCmd::SaharaDoneResp);
        let SaharaPacketBody::DoneResp(done) = second.body else {
            panic!("expected DoneResp body");
        };
        assert_eq!(done.status, 1);
    }

    #[test]
    fn rejects_truncated_header() {
        let packet = eoi_packet();
        let error = sahara_read_packet(&mut Cursor::new(&packet[..7]), false).unwrap_err();

        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn rejects_truncated_packet_body() {
        let mut packet = eoi_packet();
        packet.pop();
        let error = sahara_read_packet(&mut Cursor::new(packet), false).unwrap_err();

        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn rejects_invalid_packet_length() {
        let mut header = Vec::new();
        header.extend((SaharaCmd::SaharaDoneResp as u32).to_le_bytes());
        header.extend(7u32.to_le_bytes());

        let error = sahara_read_packet(&mut Cursor::new(header), false).unwrap_err();

        assert!(error.to_string().contains("Invalid Sahara packet length 7"));
    }
}

/// Golden wire-format vectors. They pin the exact bytes the protocol structs
/// encode to and decode from, so the (de)serialization layer can be replaced
/// without changing the wire format.
#[cfg(test)]
mod golden {
    use super::*;

    const HELLO: &str = "010000003000000002000000010000000004000002000000100000001100000012000000130000001400000015000000";
    const HELLO_RESP: &str = "020000003000000003000000010000000000000003000000200000002100000022000000230000002400000025000000";
    const READ_DATA: &str = "03000000140000000d0000000010000000020000";
    const EOI: &str = "04000000100000000d00000000000000";
    const DONE: &str = "0500000008000000";
    const DONE_RESP: &str = "060000000c00000001000000";
    const RESET: &str = "0700000008000000";
    const RESET_RESP: &str = "0800000008000000";
    const CMD_READY: &str = "0b00000008000000";
    const SWITCH_MODE: &str = "0c0000000c00000003000000";
    const EXEC: &str = "0d0000000c00000002000000";
    const EXEC_RESP: &str = "0e000000100000000200000040000000";
    const EXEC_DATA: &str = "0f0000000c00000003000000";
    const MEM_DEBUG64: &str = "100000001800000088776655443322110020000000000000";
    const MEM_READ64: &str = "110000001800000001000000ffffffff4000000000000000";
    const READ_DATA64: &str = "12000000200000000d0000000000000000000000010000000040000000000000";
    const RAMDUMP: &str = "0100000000000000000000000100000000200000000000004444525f4353302e42494e0000000000000000006464725f6373302e62696e000000000000000000";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn encode(pkt: &SaharaPacket) -> Vec<u8> {
        pkt.to_le_bytes()
    }

    fn decode_table(buf: &[u8]) -> Result<RamdumpTable64> {
        RamdumpTable64::from_le_bytes(buf)
    }

    fn pkt(cmd: SaharaCmd, len: u32, body: SaharaPacketBody) -> SaharaPacket {
        SaharaPacket { cmd, len, body }
    }

    /// (name, packet, expected bytes, can be parsed by `sahara_parse_packet`)
    fn cases() -> Vec<(&'static str, SaharaPacket, &'static str, bool)> {
        use SaharaCmd as C;
        use SaharaPacketBody as B;
        vec![
            (
                "hello",
                pkt(
                    C::SaharaHello,
                    0x30,
                    B::HelloReq(HelloReq {
                        ver: 2,
                        compatible: 1,
                        max_len: 0x400,
                        mode: SaharaMode::MemoryDebug,
                        unk0: 0x10,
                        unk1: 0x11,
                        unk2: 0x12,
                        unk3: 0x13,
                        unk4: 0x14,
                        unk5: 0x15,
                    }),
                ),
                HELLO,
                true,
            ),
            (
                "hello_resp",
                pkt(
                    C::SaharaHelloResp,
                    0x30,
                    B::HelloResp(HelloResp {
                        ver: 3,
                        compatible: 1,
                        status: 0,
                        mode: SaharaMode::Command,
                        unk0: 0x20,
                        unk1: 0x21,
                        unk2: 0x22,
                        unk3: 0x23,
                        unk4: 0x24,
                        unk5: 0x25,
                    }),
                ),
                HELLO_RESP,
                true,
            ),
            (
                "read_data",
                pkt(
                    C::SaharaReadData,
                    20,
                    B::ReadReq(ReadReq {
                        image: 13,
                        offset: 0x1000,
                        len: 0x200,
                    }),
                ),
                READ_DATA,
                true,
            ),
            (
                "eoi",
                pkt(
                    C::SaharaEndOfImage,
                    16,
                    B::Eoi(Eoi {
                        image: 13,
                        status: 0,
                    }),
                ),
                EOI,
                true,
            ),
            (
                "done",
                pkt(C::SaharaDone, 8, B::DoneReq(DoneReq {})),
                DONE,
                true,
            ),
            (
                "done_resp",
                pkt(C::SaharaDoneResp, 12, B::DoneResp(DoneResp { status: 1 })),
                DONE_RESP,
                true,
            ),
            (
                "reset",
                pkt(C::SaharaReset, 8, B::ResetReq(ResetReq {})),
                RESET,
                false,
            ),
            (
                "reset_resp",
                pkt(C::SaharaResetResp, 8, B::ResetResp(ResetResp {})),
                RESET_RESP,
                true,
            ),
            (
                "command_ready",
                pkt(C::SaharaCommandReady, 8, B::CommandReady(CommandReady {})),
                CMD_READY,
                true,
            ),
            (
                "switch_mode",
                pkt(
                    C::SaharaSwitchMode,
                    12,
                    B::SwitchMode(SwitchMode {
                        mode: SaharaMode::Command,
                    }),
                ),
                SWITCH_MODE,
                false,
            ),
            (
                "execute",
                pkt(C::SaharaExecute, 12, B::Command(SaharaCmdModeCmd::ReadHwId)),
                EXEC,
                false,
            ),
            (
                "execute_resp",
                pkt(
                    C::SaharaExecuteResp,
                    16,
                    B::ExecResp(ExecResp {
                        command: SaharaCmdModeCmd::ReadHwId,
                        len: 0x40,
                    }),
                ),
                EXEC_RESP,
                true,
            ),
            (
                "execute_data",
                pkt(
                    C::SaharaExecuteData,
                    12,
                    B::Command(SaharaCmdModeCmd::ReadOemKeyHash),
                ),
                EXEC_DATA,
                true,
            ),
            (
                "mem_debug64",
                pkt(
                    C::SaharaMemDebug64,
                    24,
                    B::Debug64Req(Debug64Req {
                        addr: 0x1122_3344_5566_7788,
                        len: 0x2000,
                    }),
                ),
                MEM_DEBUG64,
                true,
            ),
            (
                "mem_read64",
                pkt(
                    C::SaharaMemRead64,
                    24,
                    B::ReadMem64Req(ReadMem64Req {
                        addr: 0xffff_ffff_0000_0001,
                        len: 0x40,
                    }),
                ),
                MEM_READ64,
                true,
            ),
            (
                "read_data64",
                pkt(
                    C::SaharaReadData64,
                    32,
                    B::ReadData64Req(ReadData64Req {
                        image: 13,
                        offset: 0x1_0000_0000,
                        len: 0x4000,
                    }),
                ),
                READ_DATA64,
                true,
            ),
        ]
    }

    #[test]
    fn encodes_every_packet_to_golden_bytes() {
        for (name, packet, hex, _) in cases() {
            assert_eq!(encode(&packet), unhex(hex), "{name}");
        }
    }

    #[test]
    fn decodes_golden_bytes_into_every_parseable_packet() {
        for (name, packet, hex, parseable) in cases() {
            if !parseable {
                continue;
            }
            let parsed = sahara_parse_packet(&unhex(hex), false).unwrap();
            assert_eq!(format!("{parsed:?}"), format!("{packet:?}"), "{name}");
        }
    }

    #[test]
    fn decoding_ignores_trailing_bytes() {
        for (name, packet, hex, parseable) in cases() {
            if !parseable {
                continue;
            }
            let mut bytes = unhex(hex);
            bytes.extend([0xaa, 0xbb, 0xcc]);
            let parsed = sahara_parse_packet(&bytes, false).unwrap();
            assert_eq!(format!("{parsed:?}"), format!("{packet:?}"), "{name}");
        }
    }

    #[test]
    fn decoding_rejects_short_bodies() {
        for (name, _, hex, parseable) in cases() {
            let bytes = unhex(hex);
            if !parseable || bytes.len() <= 8 {
                continue;
            }
            for cut in 1..=(bytes.len() - 8) {
                assert!(
                    sahara_parse_packet(&bytes[..bytes.len() - cut], false).is_err(),
                    "{name} cut {cut}"
                );
            }
        }
        assert!(sahara_parse_packet(&[], false).is_err());
        assert!(sahara_parse_packet(&unhex(EOI)[..7], false).is_err());
    }

    #[test]
    fn decoding_rejects_unknown_command_and_mode() {
        let mut unknown_cmd = unhex(EOI);
        unknown_cmd[..4].copy_from_slice(&0x99u32.to_le_bytes());
        assert!(sahara_parse_packet(&unknown_cmd, false).is_err());

        // HelloReq.mode sits after ver/compatible/max_len.
        let mut unknown_mode = unhex(HELLO);
        unknown_mode[20..24].copy_from_slice(&9u32.to_le_bytes());
        assert!(sahara_parse_packet(&unknown_mode, false).is_err());

        // ExecResp.command is a SaharaCmdModeCmd.
        let mut unknown_exec = unhex(EXEC_RESP);
        unknown_exec[8..12].copy_from_slice(&0x77u32.to_le_bytes());
        assert!(sahara_parse_packet(&unknown_exec, false).is_err());
    }

    #[test]
    fn sahara_xml_marker_is_not_a_sahara_packet() {
        let mut bytes = b"<?xm".to_vec();
        bytes.extend([0u8; 8]);
        assert!(sahara_parse_packet(&bytes, false).is_err());
    }

    #[test]
    fn ramdump_table_entry_decodes_golden_bytes() {
        let entry = decode_table(&unhex(RAMDUMP)).unwrap();
        assert_eq!(entry.save_pref, 1);
        assert_eq!(entry.base, 0x1_0000_0000);
        assert_eq!(entry.len, 0x2000);
        assert_eq!(&entry.description[..11], b"DDR_CS0.BIN");
        assert_eq!(&entry.description[11..], &[0u8; 9]);
        assert_eq!(&entry.filename[..11], b"ddr_cs0.bin");
        assert_eq!(&entry.filename[11..], &[0u8; 9]);
        assert_eq!(size_of::<RamdumpTable64>(), unhex(RAMDUMP).len());
    }

    #[test]
    fn ramdump_table_entry_rejects_short_input() {
        let bytes = unhex(RAMDUMP);
        assert!(decode_table(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode_table(&[]).is_err());
    }
}
