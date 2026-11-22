// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Type and dimensions of images.
//!
//! The media type is recognised from the leading bytes of the content. The
//! width and height are read from the image header, without decoding the
//! pixels, for every format that the image decoder supports: PNG, JPEG, GIF,
//! WebP, BMP, TIFF, ICO, TGA, DDS, HDR, OpenEXR, PNM and QOI. Other image
//! types, such as HEIF or Photoshop documents, are recognised without
//! dimensions.

use std::fmt;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

use ::image::{ImageError as DecoderError, ImageFormat, ImageReader};
use nurt_contracts::error::ErrorCode;

/// Number of leading bytes that [`image_mime`] needs to recognise every
/// supported image type.
pub const SIGNATURE_LEN: usize = 256;

/// Recognises an image by its leading bytes and returns its media type.
///
/// # Examples
///
/// ```
/// use nurt_files::read::image::image_mime;
///
/// assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
/// assert_eq!(image_mime(b"%PDF-1.7"), None);
/// ```
pub fn image_mime(prefix: &[u8]) -> Option<&'static str> {
    infer::get(prefix)
        .filter(|kind| kind.matcher_type() == infer::MatcherType::Image)
        .map(|kind| kind.mime_type())
}

/// Media type and dimensions of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    mime: &'static str,
    dimensions: Option<(u32, u32)>,
}

impl ImageInfo {
    /// Media type, for example `image/png`.
    pub fn mime(&self) -> &'static str {
        self.mime
    }

    /// Width in pixels; `None` for a format whose header is not read.
    pub fn width(&self) -> Option<u32> {
        self.dimensions.map(|(width, _)| width)
    }

    /// Height in pixels; `None` for a format whose header is not read.
    pub fn height(&self) -> Option<u32> {
        self.dimensions.map(|(_, height)| height)
    }
}

/// Failure to inspect an image.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImageError {
    /// The content is not an image of a recognised type.
    NotImage,
    /// The image header is damaged.
    Malformed {
        /// Media type recognised from the leading bytes.
        mime: &'static str,
        /// Description of the defect.
        detail: String,
    },
    /// The content cannot be read.
    Io(io::Error),
}

impl ImageError {
    /// Error code of the language for this failure.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_files::read::image::ImageError;
    ///
    /// assert_eq!(ImageError::NotImage.code(), ErrorCode::Format);
    /// ```
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotImage | Self::Malformed { .. } => ErrorCode::Format,
            Self::Io(_) => ErrorCode::Io,
        }
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotImage => f.write_str(
                "the content is not an image of a recognised type; read it with view: hex",
            ),
            Self::Malformed { mime, detail } => write!(
                f,
                "malformed {mime} image: {detail}; check that the file is complete"
            ),
            Self::Io(error) => write!(f, "cannot read the image: {error}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::NotImage | Self::Malformed { .. } => None,
        }
    }
}

impl From<io::Error> for ImageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Recognises an image and reads its dimensions from the header.
///
/// The reader is read from its current position; only the bytes of the
/// header are consumed.
///
/// # Errors
///
/// Returns [`ImageError::NotImage`] when the leading bytes are not those of
/// an image, [`ImageError::Malformed`] when the header of a supported format
/// is damaged and [`ImageError::Io`] when the reader fails.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use nurt_files::read::image::inspect;
///
/// let mut png = Vec::new();
/// image::DynamicImage::new_rgb8(3, 2)
///     .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
/// let info = inspect(Cursor::new(png))?;
/// assert_eq!((info.mime(), info.width(), info.height()), ("image/png", Some(3), Some(2)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn inspect<R: BufRead + Seek>(mut reader: R) -> Result<ImageInfo, ImageError> {
    let start = reader.stream_position()?;
    let mut prefix = Vec::with_capacity(SIGNATURE_LEN);
    (&mut reader)
        .take(SIGNATURE_LEN as u64)
        .read_to_end(&mut prefix)?;
    let mime = image_mime(&prefix).ok_or(ImageError::NotImage)?;
    reader.seek(SeekFrom::Start(start))?;
    let Some(format) = ImageFormat::from_mime_type(mime).filter(ImageFormat::reading_enabled)
    else {
        return Ok(ImageInfo {
            mime,
            dimensions: None,
        });
    };
    let dimensions = match ImageReader::with_format(reader, format).into_dimensions() {
        Ok(dimensions) => Some(dimensions),
        Err(DecoderError::Unsupported(_)) => None,
        Err(DecoderError::IoError(error)) if error.kind() != io::ErrorKind::UnexpectedEof => {
            return Err(ImageError::Io(error));
        }
        Err(error) => {
            return Err(ImageError::Malformed {
                mime,
                detail: error.to_string(),
            });
        }
    };
    Ok(ImageInfo { mime, dimensions })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::image::DynamicImage;

    use super::*;

    fn encoded(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn reads_type_and_dimensions_of_common_formats() {
        let cases = [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Gif, "image/gif"),
            (ImageFormat::Bmp, "image/bmp"),
            (ImageFormat::Tiff, "image/tiff"),
            (ImageFormat::WebP, "image/webp"),
        ];
        for (format, mime) in cases {
            let info = inspect(Cursor::new(encoded(format, 7, 5))).unwrap();
            assert_eq!(info.mime(), mime);
            assert_eq!((info.width(), info.height()), (Some(7), Some(5)), "{mime}");
        }
    }

    #[test]
    fn reads_from_current_position() {
        let mut bytes = b"prefix".to_vec();
        bytes.extend(encoded(ImageFormat::Png, 4, 9));
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(6);
        assert_eq!(inspect(cursor).unwrap().height(), Some(9));
    }

    #[test]
    fn recognised_type_without_decoder_has_no_dimensions() {
        let mut photoshop = b"8BPS\x00\x01".to_vec();
        photoshop.resize(64, 0);
        let info = inspect(Cursor::new(photoshop)).unwrap();
        assert_eq!(info.mime(), "image/vnd.adobe.photoshop");
        assert_eq!(info.width(), None);
    }

    #[test]
    fn truncated_header_is_malformed() {
        let png = encoded(ImageFormat::Png, 3, 3);
        let error = inspect(Cursor::new(png[..12].to_vec())).unwrap_err();
        assert!(
            matches!(
                error,
                ImageError::Malformed {
                    mime: "image/png",
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(error.code(), ErrorCode::Format);
        assert!(error.to_string().starts_with("malformed image/png image: "));
    }

    #[test]
    fn text_is_not_an_image() {
        let error = inspect(Cursor::new(b"plain text".to_vec())).unwrap_err();
        assert!(matches!(error, ImageError::NotImage));
        assert!(error.to_string().contains("view: hex"));
        assert!(std::error::Error::source(&error).is_none());
    }

    #[test]
    fn read_failure_is_io_error() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("device error"))
            }
        }
        impl BufRead for Failing {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                Err(io::Error::other("device error"))
            }
            fn consume(&mut self, _: usize) {}
        }
        impl Seek for Failing {
            fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
                Ok(0)
            }
        }
        let error = inspect(Failing).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
        assert!(std::error::Error::source(&error).is_some());
        assert_eq!(error.to_string(), "cannot read the image: device error");
    }

    #[test]
    fn mime_requires_image_signature() {
        assert_eq!(image_mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(image_mime(b"PK\x03\x04"), None);
        assert_eq!(image_mime(b""), None);
    }
}
