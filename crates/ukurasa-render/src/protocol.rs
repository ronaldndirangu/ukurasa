//! Messages exchanged with the worker process over stdin and stdout.
//!
//! Each frame is a little-endian `u32` header length, a JSON header, a little-endian `u32`
//! payload length, and the raw payload bytes.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const MAX_HEADER_LEN: usize = 16 << 20;
const MAX_PAYLOAD_LEN: usize = 512 << 20;

/// Page dimensions in PDF points, with the page rotation applied.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// What the reader needs to know about a newly opened document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub pages: Vec<PageSize>,
    #[serde(default)]
    pub outline: Vec<OutlineItem>,
}

/// A bookmark in the document outline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineItem {
    pub title: String,
    /// Zero-based target page, when the entry points into this document.
    pub page: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<OutlineItem>,
}

/// A clickable area on a page, in display points with a top-left origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub target: LinkTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkTarget {
    /// A zero-based page in this document; `top` is the target's distance in display points
    /// from the top of that page, when the destination names one.
    Page { page: u32, top: Option<f32> },
    /// A URI exactly as written in the file.
    Uri { uri: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Open {
        path: PathBuf,
    },
    /// Renders a zero-based page at the given pixel width; the reply payload is a PNG.
    Render {
        page: u32,
        width: u32,
    },
    /// Renders from the PDF in the payload, an edited copy of the open document, until the
    /// next open or revision. An empty payload returns to the file.
    Revise,
    /// Lists the links on a zero-based page of the document being rendered.
    Links {
        page: u32,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Opened(DocumentInfo),
    Rendered { width: u32, height: u32 },
    Revised,
    Links { links: Vec<Link> },
    Closed,
    Error { kind: ErrorKind, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Io,
    Format,
    Password,
    Unsupported,
    Page,
    Internal,
}

pub fn write_frame<T: Serialize>(
    writer: &mut impl Write,
    header: &T,
    payload: &[u8],
) -> io::Result<()> {
    let header = serde_json::to_vec(header).map_err(io::Error::other)?;
    write_chunk(writer, &header, MAX_HEADER_LEN)?;
    write_chunk(writer, payload, MAX_PAYLOAD_LEN)?;
    writer.flush()
}

/// Reads one frame. Fails with [`io::ErrorKind::UnexpectedEof`] when the stream has ended.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<(T, Vec<u8>)> {
    let header = read_chunk(reader, MAX_HEADER_LEN)?;
    let payload = read_chunk(reader, MAX_PAYLOAD_LEN)?;
    let header = serde_json::from_slice(&header)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((header, payload))
}

fn write_chunk(writer: &mut impl Write, bytes: &[u8], limit: usize) -> io::Result<()> {
    if bytes.len() > limit {
        return Err(too_large(bytes.len()));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)
}

fn read_chunk(reader: &mut impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut len = [0; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > limit {
        return Err(too_large(len));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn too_large(len: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("frame of {len} bytes exceeds the protocol limit"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut stream = Vec::new();
        let request = Request::Render {
            page: 3,
            width: 800,
        };
        write_frame(&mut stream, &request, b"payload").unwrap();
        write_frame(&mut stream, &Request::Close, &[]).unwrap();

        let mut reader = stream.as_slice();
        let (first, payload): (Request, _) = read_frame(&mut reader).unwrap();
        assert_eq!(first, request);
        assert_eq!(payload, b"payload");
        let (second, _): (Request, _) = read_frame(&mut reader).unwrap();
        assert_eq!(second, Request::Close);

        let end = read_frame::<Request>(&mut reader).unwrap_err();
        assert_eq!(end.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn json_uses_tagged_snake_case() {
        let response = Response::Error {
            kind: ErrorKind::Password,
            message: "locked".into(),
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"type":"error","kind":"password","message":"locked"}"#
        );
    }

    #[test]
    fn opened_documents_round_trip_with_outlines() {
        let response = Response::Opened(DocumentInfo {
            pages: vec![PageSize {
                width: 612.0,
                height: 792.0,
            }],
            outline: vec![OutlineItem {
                title: "Intro".into(),
                page: Some(0),
                children: vec![OutlineItem {
                    title: "External".into(),
                    page: None,
                    children: Vec::new(),
                }],
            }],
        });
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.starts_with(r#"{"type":"opened","pages":"#), "{json}");
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);

        let bare: Response = serde_json::from_str(r#"{"type":"opened","pages":[]}"#).unwrap();
        assert_eq!(bare, Response::Opened(DocumentInfo::default()));
    }

    #[test]
    fn links_round_trip() {
        let response = Response::Links {
            links: vec![
                Link {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 12.0,
                    target: LinkTarget::Page {
                        page: 4,
                        top: Some(100.0),
                    },
                },
                Link {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                    target: LinkTarget::Uri {
                        uri: "https://example.com".into(),
                    },
                },
            ],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(
            json.contains(r#""target":{"kind":"page","page":4,"top":100.0}"#),
            "{json}"
        );
        assert!(
            json.contains(r#"{"kind":"uri","uri":"https://example.com"}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
    }

    #[test]
    fn oversized_frames_are_rejected() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&u32::MAX.to_le_bytes());
        let error = read_frame::<Request>(&mut stream.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
