mod encoding;
mod header;
mod language;
mod loader;
pub mod read;
mod version;
mod windows_version;

use byteorder::{ReadBytesExt, LE};
use color_eyre::eyre::{bail, eyre, OptionExt};
use color_eyre::Result;
use flate2::read::ZlibDecoder;
use liblzma::read::XzDecoder;
use msi::Language;
use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::mem;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use yara_x::mods::pe::ResourceType;
use yara_x::mods::PE;

use crate::installers::inno::header::Header;
use crate::installers::inno::language::LanguageEntry;
use crate::installers::inno::loader::{SetupLoader, SETUP_LOADER_RESOURCE};
use crate::installers::inno::read::block_filter::{InnoBlockFilter, INNO_BLOCK_SIZE};
use crate::installers::inno::read::crc32::Crc32Reader;
use crate::installers::inno::version::{InnoVersion, KnownVersion};
use crate::installers::utils::read_lzma_stream_header;
use crate::manifests::installer_manifest::{ElevationRequirement, UnsupportedOSArchitecture};
use crate::types::architecture::Architecture;
use crate::types::language_tag::LanguageTag;

const VERSION_LEN: usize = 1 << 6;

const MAX_SUPPORTED_VERSION: InnoVersion = InnoVersion(6, 3, u8::MAX);

pub struct InnoFile {
    pub architecture: Option<Architecture>,
    pub unsupported_architectures: Option<BTreeSet<UnsupportedOSArchitecture>>,
    pub uninstall_name: Option<String>,
    pub app_version: Option<String>,
    pub app_publisher: Option<String>,
    pub product_code: Option<String>,
    pub elevation_requirement: Option<ElevationRequirement>,
    pub installer_locale: Option<LanguageTag>,
}

impl InnoFile {
    pub fn new(data: &[u8], pe: &PE) -> Result<Self> {
        let setup_loader_data = pe
            .resources
            .iter()
            .filter(|resource| resource.type_() == ResourceType::RESOURCE_TYPE_RCDATA)
            .find(|resource| resource.id() == SETUP_LOADER_RESOURCE)
            .and_then(|resource| {
                let offset = resource.offset() as usize;
                data.get(offset..offset + resource.length() as usize)
            })
            .ok_or_eyre("No setup loader resource was found")?;

        let setup_loader = SetupLoader::new(setup_loader_data)?;

        let header_offset = setup_loader.header_offset as usize;
        let version_bytes = data
            .get(header_offset..header_offset + VERSION_LEN)
            .and_then(|bytes| memchr::memchr(0, bytes).map(|len| &bytes[..len]))
            .ok_or_eyre("Invalid Inno header version")?;

        let known_version = KnownVersion::from_version_bytes(version_bytes).ok_or_else(|| {
            eyre!(
                "Unknown Inno Setup version: {}",
                String::from_utf8_lossy(version_bytes)
            )
        })?;

        if known_version > MAX_SUPPORTED_VERSION {
            bail!("Inno Setup version {known_version} is newer than the maximum supported version {MAX_SUPPORTED_VERSION}");
        }

        let mut cursor = Cursor::new(data);
        cursor.set_position((header_offset + VERSION_LEN) as u64);

        let expected_checksum = cursor.read_u32::<LE>()?;

        let mut actual_checksum = Crc32Reader::new(&mut cursor);

        let stored_size = if known_version > InnoVersion(4, 0, 9) {
            let size = actual_checksum.read_u32::<LE>()?;
            let compressed = actual_checksum.read_u8()?;

            if compressed != 0 {
                if known_version > InnoVersion(4, 1, 6) {
                    Compression::LZMA1(size)
                } else {
                    Compression::Zlib(size)
                }
            } else {
                Compression::Stored(size)
            }
        } else {
            let compressed_size = actual_checksum.read_u32::<LE>()?;
            let uncompressed_size = actual_checksum.read_u32::<LE>()?;

            let mut stored_size = if compressed_size == u32::MAX {
                Compression::Stored(uncompressed_size)
            } else {
                Compression::Zlib(compressed_size)
            };

            // Add the size of a CRC32 checksum for each 4KiB sub-block
            *stored_size += stored_size.div_ceil(u32::from(INNO_BLOCK_SIZE)) * 4;

            stored_size
        };

        let actual_checksum = actual_checksum.finalize();
        if actual_checksum != expected_checksum {
            bail!(
                "CRC32 checksum mismatch. Actual: {actual_checksum}. Expected: {expected_checksum}."
            );
        }

        let mut block_filter = InnoBlockFilter::new(cursor.take(u64::from(*stored_size)));
        let mut reader: Box<dyn Read> = match stored_size {
            Compression::LZMA1(_) => {
                let stream = read_lzma_stream_header(&mut block_filter)?;
                Box::new(XzDecoder::new_stream(block_filter, stream))
            }
            Compression::Zlib(_) => Box::new(ZlibDecoder::new(block_filter)),
            Compression::Stored(_) => Box::new(block_filter),
        };

        let mut header = Header::load(&mut reader, &known_version)?;

        let mut language_entries = Vec::new();
        for _ in 0..header.language_count {
            language_entries.push(LanguageEntry::load(&mut reader, &known_version)?);
        }

        Ok(Self {
            architecture: mem::take(&mut header.architectures_allowed).to_winget_architecture(),
            unsupported_architectures: mem::take(&mut header.architectures_disallowed)
                .to_unsupported_architectures(),
            uninstall_name: header.uninstall_name.take(),
            app_version: header.app_version.take(),
            app_publisher: header.app_publisher.take(),
            product_code: header.app_id.take().map(to_product_code),
            elevation_requirement: header
                .privileges_required
                .to_elevation_requirement(&header.privileges_required_overrides_allowed),
            installer_locale: language_entries.first().and_then(|language_entry| {
                LanguageTag::from_str(
                    Language::from_code(u16::try_from(language_entry.language_id).ok()?).tag(),
                )
                .ok()
            }),
        })
    }
}

pub fn to_product_code(mut app_id: String) -> String {
    // Remove escaped bracket
    if app_id.starts_with("{{") {
        app_id.remove(0);
    }

    // Inno tags '_is1' onto the end of the app_id to create the Uninstall registry key
    app_id.push_str("_is1");
    app_id
}

enum Compression {
    Stored(u32),
    Zlib(u32),
    LZMA1(u32),
}

impl Deref for Compression {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Stored(size) | Self::Zlib(size) | Self::LZMA1(size) => size,
        }
    }
}

impl DerefMut for Compression {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Stored(size) | Self::Zlib(size) | Self::LZMA1(size) => size,
        }
    }
}
