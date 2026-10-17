use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Display,
    EnumIter,
    EnumString,
    IntoStaticStr,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum AttachmentParser {
    Image,
    Audio,
    Video,
    Ocr,
}

impl AttachmentParser {
    pub fn name(&self) -> &'static str {
        self.into()
    }

    /// The parser a byte prefix declares: image magic first, then container magic.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if ImageFormat::sniff(bytes).is_some() {
            return Some(Self::Image);
        }
        MediaCodec::sniff(bytes).map(Self::of_codec)
    }

    /// The parser a file name declares, by extension.
    pub fn from_filename(name: &str) -> Option<Self> {
        if ImageFormat::from_filename(name).is_some() {
            return Some(Self::Image);
        }
        MediaCodec::from_filename(name).map(Self::of_codec)
    }

    /// The parser that reads a container of this codec.
    fn of_codec(codec: MediaCodec) -> Self {
        if codec.is_audio() {
            Self::Audio
        } else {
            Self::Video
        }
    }
}

/// What one attachment row is; the name is the `<attachment type="…">` value and the column value.
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Display,
    EnumIter,
    EnumString,
    IntoStaticStr,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Photo,
    Video,
    Audio,
    Voice,
    Document,
    Sticker,
    Animation,
    VideoNote,
}

impl AttachmentKind {
    pub fn name(&self) -> &'static str {
        self.into()
    }
}

enum_column!(AttachmentKind);
enum_column!(AttachmentParser);

/// Platform extras carried by one attachment rather than by the message.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttachmentMeta {
    /// The text an app-generated attachment was produced from (speech script, image prompt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// A sticker's own emoji.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The codec the platform's own media type declared, when it declared one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_codec: Option<MediaCodec>,
}

impl sqlx::Type<sqlx::Postgres> for AttachmentMeta {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <serde_json::Value as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <serde_json::Value as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for AttachmentMeta {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> std::result::Result<Self, sqlx::error::BoxDynError> {
        let raw = <serde_json::Value as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
        serde_json::from_value(raw)
            .map_err(|e| format!("attachment meta cannot be read: {e}").into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString, IntoStaticStr, Serialize, Deserialize)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
#[serde(rename_all = "lowercase")]
pub enum MediaCodec {
    Mp3,
    Ogg,
    Wav,
    M4a,
    Aac,
    Flac,
    Wma,
    Mp4,
    Mov,
    Avi,
    Mkv,
    Webm,
}

/// MPEG audio frame syncs: the first two bytes of an MP3 frame.
const MP3_FRAME_SYNCS: [&[u8]; 4] = [b"\xFF\xF2", b"\xFF\xF3", b"\xFF\xFB", b"\xFF\xE3"];

/// The `ftyp` brands an MP4 container carries.
const MP4_BRANDS: [&[u8]; 12] = [
    b"isom", b"iso2", b"iso4", b"iso5", b"mp41", b"mp42", b"avc1", b"dash", b"MSNV", b"3gp4",
    b"3gp5", b"3g2a",
];

/// `ftyp` brands of an image container: `ImageFormat::sniff` reads the HEIC ones, and these are the
/// ones it leaves for the media table to mistake for video.
const IMAGE_FTYP_BRANDS: [&[u8]; 4] = [b"mif1", b"msf1", b"avif", b"avis"];

impl MediaCodec {
    pub const fn default_audio() -> Self {
        Self::Wav
    }

    pub fn ext(self) -> &'static str {
        self.into()
    }

    /// Provider `format` field on OpenAI-compatible endpoints; same strings as `ext`.
    pub fn api_format(self) -> &'static str {
        self.ext()
    }

    pub fn from_ext(ext: &str) -> Option<Self> {
        Self::from_str(ext).ok()
    }

    pub fn from_filename(name: &str) -> Option<Self> {
        Self::from_ext(extension(name))
    }

    /// The codec an IANA media subtype names, for the subtypes whose spelling is not the extension.
    pub fn from_mime(subtype: &str) -> Option<Self> {
        match subtype {
            "mpeg" => Some(Self::Mp3),
            "x-wav" | "vnd.wave" => Some(Self::Wav),
            "x-m4a" => Some(Self::M4a),
            "quicktime" => Some(Self::Mov),
            "x-msvideo" => Some(Self::Avi),
            "x-matroska" => Some(Self::Mkv),
            other => Self::from_ext(other),
        }
    }

    /// The codec a byte prefix declares, by container magic.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        let has = |at: usize, tag: &[u8]| bytes.get(at..at + tag.len()) == Some(tag);
        let brand_is = |brands: &[&[u8]]| brands.iter().any(|brand| has(8, brand));

        if has(0, b"ID3") || MP3_FRAME_SYNCS.iter().any(|sync| has(0, sync)) {
            return Some(Self::Mp3);
        }
        if has(0, b"\xFF\xF1") || has(0, b"\xFF\xF9") {
            return Some(Self::Aac);
        }
        if has(0, b"OggS") {
            return Some(Self::Ogg);
        }
        if has(0, b"RIFF") && has(8, b"WAVE") {
            return Some(Self::Wav);
        }
        if has(0, b"RIFF") && has(8, b"AVI ") {
            return Some(Self::Avi);
        }
        if has(0, b"fLaC") {
            return Some(Self::Flac);
        }
        if has(0, b"\x1A\x45\xDF\xA3") {
            return Some(Self::Webm);
        }
        if has(4, b"ftyp") {
            if has(8, b"M4A ") || has(8, b"M4B ") {
                return Some(Self::M4a);
            }
            if has(8, b"qt  ") {
                return Some(Self::Mov);
            }
            if brand_is(&HEIC_BRANDS) || brand_is(&IMAGE_FTYP_BRANDS) {
                return None;
            }
            if brand_is(&MP4_BRANDS) {
                return Some(Self::Mp4);
            }
        }

        None
    }

    pub fn is_audio(self) -> bool {
        matches!(
            self,
            Self::Mp3 | Self::Ogg | Self::Wav | Self::M4a | Self::Aac | Self::Flac | Self::Wma
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum ImageFormat {
    Png,
    #[strum(serialize = "jpg", serialize = "jpeg")]
    Jpeg,
    Webp,
    Gif,
    Bmp,
    Heic,
}

const HEIC_BRANDS: [&[u8]; 8] = [
    b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs",
];

impl ImageFormat {
    /// MIME type handed to the provider (data URL prefix / `image_url` value).
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Heic => "image/heic",
        }
    }

    pub fn from_ext(ext: &str) -> Option<Self> {
        Self::from_str(ext).ok()
    }

    pub fn from_filename(name: &str) -> Option<Self> {
        Self::from_ext(extension(name))
    }

    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        let has = |at: usize, tag: &[u8]| bytes.get(at..at + tag.len()) == Some(tag);
        let bmp_declared_len = bytes
            .get(2..6)
            .and_then(|len| len.try_into().ok())
            .map(u32::from_le_bytes);

        if has(0, b"\x89PNG\r\n\x1a\n") {
            return Some(Self::Png);
        }
        if has(0, b"\xFF\xD8\xFF") {
            return Some(Self::Jpeg);
        }
        if has(0, b"GIF87a") || has(0, b"GIF89a") {
            return Some(Self::Gif);
        }
        if has(0, b"RIFF") && has(8, b"WEBP") {
            return Some(Self::Webp);
        }
        if has(0, b"BM") && bmp_declared_len == Some(bytes.len() as u32) {
            return Some(Self::Bmp);
        }
        if has(4, b"ftyp") && HEIC_BRANDS.iter().any(|brand| has(8, brand)) {
            return Some(Self::Heic);
        }

        None
    }
}

/// The extension of a name; a name with no dot is read as the extension itself.
fn extension(name: &str) -> &str {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_format_from_extension() {
        assert_eq!(
            ImageFormat::from_filename("shot.PNG"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::from_filename("/tmp/a.jpeg"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::from_filename("a.jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_ext("WebP"), Some(ImageFormat::Webp));
        assert_eq!(ImageFormat::from_filename("notes.txt"), None);
        assert_eq!(ImageFormat::from_filename("/tmp/no-extension"), None);
    }

    #[test]
    fn image_format_mime_pairs_with_every_variant() {
        assert_eq!(ImageFormat::Png.mime(), "image/png");
        assert_eq!(ImageFormat::Jpeg.mime(), "image/jpeg");
        assert_eq!(ImageFormat::Webp.mime(), "image/webp");
        assert_eq!(ImageFormat::Gif.mime(), "image/gif");
        assert_eq!(ImageFormat::Bmp.mime(), "image/bmp");
        assert_eq!(ImageFormat::Heic.mime(), "image/heic");
    }

    #[test]
    fn image_format_sniff() {
        assert_eq!(
            ImageFormat::sniff(b"\x89PNG\r\n\x1a\nrest"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::sniff(b"\xFF\xD8\xFF\xE0rest"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::sniff(b"GIF89arest"), Some(ImageFormat::Gif));
        assert_eq!(ImageFormat::sniff(b"GIF87arest"), Some(ImageFormat::Gif));
        assert_eq!(
            ImageFormat::sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some(ImageFormat::Webp)
        );
        assert_eq!(
            ImageFormat::sniff(b"\x00\x00\x00\x18ftypheicrest"),
            Some(ImageFormat::Heic)
        );
    }

    #[test]
    fn image_format_sniff_rejects_near_misses_and_short_input() {
        assert_eq!(ImageFormat::sniff(b""), None);
        assert_eq!(ImageFormat::sniff(b"\x89PNG"), None);
        assert_eq!(ImageFormat::sniff(b"RIFF\x00\x00\x00\x00AVI LIST"), None);
        assert_eq!(ImageFormat::sniff(b"BMW is a car maker"), None);
        assert_eq!(ImageFormat::sniff(b"\x00\x00\x00\x18ftypmif1rest"), None);
        assert_eq!(ImageFormat::sniff(b"\x00\x00\x00\x18ftypavifrest"), None);
    }

    #[test]
    fn image_format_sniff_accepts_bmp_with_matching_length() {
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&[0; 30]);
        assert_eq!(ImageFormat::sniff(&bmp), Some(ImageFormat::Bmp));
    }

    #[test]
    fn media_codec_reads_the_mime_aliases() {
        assert_eq!(MediaCodec::from_mime("mpeg"), Some(MediaCodec::Mp3));
        assert_eq!(MediaCodec::from_mime("x-wav"), Some(MediaCodec::Wav));
        assert_eq!(MediaCodec::from_mime("quicktime"), Some(MediaCodec::Mov));
        assert_eq!(MediaCodec::from_mime("mp4"), Some(MediaCodec::Mp4));
        assert_eq!(MediaCodec::from_mime("ogg"), Some(MediaCodec::Ogg));
        assert_eq!(MediaCodec::from_mime("octet-stream"), None);
    }

    #[test]
    fn media_codec_sniff_reads_container_magic() {
        assert_eq!(
            MediaCodec::sniff(b"OggS\x00\x02rest"),
            Some(MediaCodec::Ogg)
        );
        assert_eq!(MediaCodec::sniff(b"ID3\x04\x00"), Some(MediaCodec::Mp3));
        assert_eq!(
            MediaCodec::sniff(b"\xFF\xFB\x90\x00"),
            Some(MediaCodec::Mp3)
        );
        assert_eq!(
            MediaCodec::sniff(b"RIFF\x00\x00\x00\x00WAVEfmt "),
            Some(MediaCodec::Wav)
        );
        assert_eq!(
            MediaCodec::sniff(b"RIFF\x00\x00\x00\x00AVI LIST"),
            Some(MediaCodec::Avi)
        );
        assert_eq!(MediaCodec::sniff(b"fLaC\x00\x00"), Some(MediaCodec::Flac));
        assert_eq!(
            MediaCodec::sniff(b"\x00\x00\x00\x18ftypisomrest"),
            Some(MediaCodec::Mp4)
        );
        assert_eq!(
            MediaCodec::sniff(b"\x00\x00\x00\x18ftypM4A rest"),
            Some(MediaCodec::M4a)
        );
        assert_eq!(
            MediaCodec::sniff(b"\x00\x00\x00\x14ftypqt  rest"),
            Some(MediaCodec::Mov)
        );
    }

    #[test]
    fn media_codec_sniff_rejects_image_containers_and_near_misses() {
        assert_eq!(MediaCodec::sniff(b""), None);
        assert_eq!(MediaCodec::sniff(b"Ogg"), None);
        assert_eq!(MediaCodec::sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), None);
        // An image container must not fall through to the media table.
        assert_eq!(MediaCodec::sniff(b"\x00\x00\x00\x18ftypheicrest"), None);
        assert_eq!(MediaCodec::sniff(b"\x00\x00\x00\x18ftypmif1rest"), None);
        assert_eq!(MediaCodec::sniff(b"\x00\x00\x00\x18ftypavifrest"), None);
        // An unknown brand is not guessed at.
        assert_eq!(MediaCodec::sniff(b"\x00\x00\x00\x18ftypzzzzrest"), None);
    }

    #[test]
    fn attachment_parser_reads_bytes_before_names() {
        assert_eq!(
            AttachmentParser::from_bytes(b"\x00\x00\x00\x18ftypisomrest"),
            Some(AttachmentParser::Video)
        );
        assert_eq!(
            AttachmentParser::from_bytes(b"\x89PNG\r\n\x1a\nrest"),
            Some(AttachmentParser::Image)
        );
        assert_eq!(
            AttachmentParser::from_bytes(b"OggS\x00\x02rest"),
            Some(AttachmentParser::Audio)
        );
        // A PNG named `a.mp4`: the caller tries `from_bytes` first, so content wins.
        assert_eq!(
            AttachmentParser::from_filename("a.mp4"),
            Some(AttachmentParser::Video)
        );
    }

    #[test]
    fn attachment_parser_from_filename_maps_the_media_extensions() {
        assert_eq!(
            AttachmentParser::from_filename("a.png"),
            Some(AttachmentParser::Image)
        );
        assert_eq!(
            AttachmentParser::from_filename("a.mp3"),
            Some(AttachmentParser::Audio)
        );
        assert_eq!(
            AttachmentParser::from_filename("a.ogg"),
            Some(AttachmentParser::Audio)
        );
        assert_eq!(
            AttachmentParser::from_filename("clip.mov"),
            Some(AttachmentParser::Video)
        );
        assert_eq!(AttachmentParser::from_filename("notes.txt"), None);
        assert_eq!(AttachmentParser::from_filename("no-extension"), None);
    }
}
