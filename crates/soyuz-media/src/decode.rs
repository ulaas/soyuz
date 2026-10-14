//! The decoding building blocks the safe modules compose: RAII wrappers over the FFmpeg
//! objects, implemented under `sys/` (the only place with `unsafe`) and re-exported here
//! for `info`, `frame`, `audio` and the later `player` / `transcode`.
//!
//! - [`Demuxer`]: an opened input (custom AVIO over a [`crate::Source`], [`Limits`]
//!   applied before the open); streams as [`StreamRef`]; `read` / `seek_before`.
//! - [`Packet`]: one `AVPacket`.
//! - [`Decoder`]: an open codec context, configured through AVOptions on top of
//!   `avcodec_parameters_to_context` (never a struct write); `send` / `receive`.
//! - [`Frame`]: one `AVFrame`, read through the loaded major's offset table.
//! - [`Scaler`]: picture to RGBA; [`Resampler`]: audio to interleaved `f32`.
//! - [`ChLayout`]: an owned `AVChannelLayout` (channel count and description).

pub(crate) use crate::sys::codec::{ChLayout, Decoder, Frame, Recv};
pub(crate) use crate::sys::demux::{Demuxer, Limits, Packet, StreamRef};
pub(crate) use crate::sys::resample::Resampler;
pub(crate) use crate::sys::scale::{Quality, Scaler};
