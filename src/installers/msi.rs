use crate::installers::utils::{
    RELATIVE_APP_DATA, RELATIVE_COMMON_FILES_32, RELATIVE_COMMON_FILES_64, RELATIVE_LOCAL_APP_DATA,
    RELATIVE_PROGRAM_FILES_32, RELATIVE_PROGRAM_FILES_64, RELATIVE_TEMP_FOLDER,
    RELATIVE_WINDOWS_DIR,
};
use crate::manifests::installer_manifest::{
    AppsAndFeaturesEntry, InstallationMetadata, Installer, Scope,
};
use crate::types::architecture::Architecture;
use crate::types::installer_type::InstallerType;
use crate::types::language_tag::LanguageTag;
use crate::types::version::Version;
use camino::Utf8PathBuf;
use msi::{Language, Package, Select};
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Read, Result, Seek};
use std::str::{FromStr, SplitAsciiWhitespace};

const PROPERTY: &str = "Property";
const CONTROL: &str = "Control";

const PRODUCT_CODE: &str = "ProductCode";
const PRODUCT_LANGUAGE: &str = "ProductLanguage";
const PRODUCT_NAME: &str = "ProductName";
const PRODUCT_VERSION: &str = "ProductVersion";
const MANUFACTURER: &str = "Manufacturer";
const UPGRADE_CODE: &str = "UpgradeCode";
const ALL_USERS: &str = "ALLUSERS";
const WIX: &[u8; 3] = b"Wix";
const WINDOWS_INSTALLER_XML: &[u8; 21] = b"Windows Installer XML";
const GOOGLE_CHROME: &str = "Google Chrome";

const INSTALL_DIR: &str = "INSTALLDIR";
const TARGET_DIR: &str = "TARGETDIR";

pub struct Msi {
    pub installer: Installer,
}

impl Msi {
    pub fn new<R: Read + Seek>(reader: R) -> Result<Self> {
        let mut msi = Package::open(reader)?;

        let architecture = match msi.summary_info().arch() {
            Some("x64" | "Intel64" | "AMD64") => Architecture::X64,
            Some("Intel") | None => Architecture::X86,
            Some("Arm64") => Architecture::Arm64,
            Some("Arm") => Architecture::Arm,
            Some(arch) => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(r#"Unknown MSI architecture: "{arch}""#),
                ))
            }
        };

        let mut property_table = Self::get_property_table(&mut msi)?;

        let product_name = property_table.remove(PRODUCT_NAME);
        let manufacturer = property_table.remove(MANUFACTURER);
        let product_version = product_name
            .as_deref()
            .is_some_and(|product_name| product_name == GOOGLE_CHROME)
            .then(|| Self::get_actual_chrome_version(&msi).map(str::to_owned))
            .unwrap_or_else(|| property_table.remove(PRODUCT_VERSION));
        let product_code = property_table.remove(PRODUCT_CODE);
        let upgrade_code = property_table.remove(UPGRADE_CODE);

        // https://learn.microsoft.com/windows/win32/msi/allusers
        let all_users = match property_table.remove(ALL_USERS).as_deref() {
            Some("1") => Some(Scope::Machine),
            Some("2") => None, // Installs depending on installation context and user privileges
            Some("") => Some(Scope::User), // An empty string specifies per-user context
            _ => {
                if msi
                    .select_rows(Select::table(CONTROL).columns(&[PROPERTY]))
                    .is_ok_and(|mut rows| rows.any(|row| row[0].as_str() == Some(ALL_USERS)))
                {
                    // ALLUSERS could be changed at runtime
                    None
                } else {
                    // No value or control specifies per-user context
                    Some(Scope::User)
                }
            }
        };

        Ok(Self {
            installer: Installer {
                locale: property_table.remove(PRODUCT_LANGUAGE).and_then(|code| {
                    LanguageTag::from_str(Language::from_code(code.parse::<u16>().ok()?).tag()).ok()
                }),
                architecture,
                r#type: Self::is_wix(&msi, &property_table)
                    .then_some(InstallerType::Wix)
                    .or(Some(InstallerType::Msi)),
                scope: all_users,
                product_code: product_code.clone(),
                apps_and_features_entries: [
                    &product_name,
                    &manufacturer,
                    &product_version,
                    &upgrade_code,
                ]
                .iter()
                .any(|option| option.is_some())
                .then(|| {
                    vec![AppsAndFeaturesEntry {
                        display_name: product_name,
                        publisher: manufacturer,
                        display_version: product_version.as_deref().map(Version::new),
                        product_code,
                        upgrade_code,
                        ..AppsAndFeaturesEntry::default()
                    }]
                }),
                installation_metadata: Some(InstallationMetadata {
                    default_install_location: Self::find_install_directory(
                        &Self::get_directory_table(&mut msi)?,
                        &property_table,
                    ),
                    ..InstallationMetadata::default()
                }),
                ..Installer::default()
            },
        })
    }

    fn is_wix<R: Read + Seek>(msi: &Package<R>, property_table: &HashMap<String, String>) -> bool {
        msi.summary_info()
            .creating_application()
            .map(str::as_bytes)
            .is_some_and(|app| {
                app.windows(WIX.len())
                    .any(|window| window.eq_ignore_ascii_case(WIX))
                    || app
                        .windows(WINDOWS_INSTALLER_XML.len())
                        .any(|window| window.eq_ignore_ascii_case(WINDOWS_INSTALLER_XML))
            })
            || property_table.iter().any(|(property, value)| {
                property
                    .as_bytes()
                    .windows(WIX.len())
                    .any(|window| window.eq_ignore_ascii_case(WIX))
                    || value
                        .as_bytes()
                        .windows(WIX.len())
                        .any(|window| window.eq_ignore_ascii_case(WIX))
            })
    }

    /// <https://learn.microsoft.com/windows/win32/msi/property-table>
    fn get_property_table<R: Read + Seek>(msi: &mut Package<R>) -> Result<HashMap<String, String>> {
        const VALUE: &str = "Value";

        Ok(msi
            .select_rows(Select::table(PROPERTY))?
            .filter_map(|row| match (row[PROPERTY].as_str(), row[VALUE].as_str()) {
                (Some(property), Some(value)) => Some((property.to_owned(), value.to_owned())),
                _ => None,
            })
            .collect::<HashMap<_, _>>())
    }

    /// <https://learn.microsoft.com/windows/win32/msi/directory-table>
    fn get_directory_table<R: Read + Seek>(
        msi: &mut Package<R>,
    ) -> Result<HashMap<String, (Option<String>, String)>> {
        const DIRECTORY: &str = "Directory";
        const DIRECTORY_PARENT: &str = "Directory_Parent";
        const DEFAULT_DIR: &str = "DefaultDir";

        Ok(msi
            .select_rows(Select::table(DIRECTORY))?
            .filter_map(|row| {
                match (
                    row[DIRECTORY].as_str().map(str::to_owned),
                    row[DIRECTORY_PARENT].as_str().map(str::to_owned),
                    row[DEFAULT_DIR].as_str().map(|default_dir| {
                        default_dir
                            .split_once('|')
                            .map_or(default_dir, |(_, long_dir)| long_dir)
                            .to_owned()
                    }),
                ) {
                    (Some(directory), parent, Some(default)) => {
                        Some((directory, (parent, default)))
                    }
                    _ => None,
                }
            })
            .collect::<HashMap<_, _>>())
    }

    fn find_install_directory(
        directory_table: &HashMap<String, (Option<String>, String)>,
        property_table: &HashMap<String, String>,
    ) -> Option<Utf8PathBuf> {
        Self::build_directory(directory_table, INSTALL_DIR, TARGET_DIR)
            .or_else(|| {
                // Fallback: check the value of the `WIXUI_INSTALLDIR` property
                const WIX_UI_INSTALL_DIR: &str = "WIXUI_INSTALLDIR";

                property_table
                    .get(WIX_UI_INSTALL_DIR)
                    .and_then(|wix_install_dir| {
                        Self::build_directory(directory_table, wix_install_dir, TARGET_DIR)
                    })
            })
            .or_else(|| {
                // Fallback: check for an `APPDIR` directory entry
                const APP_DIR: &str = "APPDIR";

                Self::build_directory(directory_table, APP_DIR, TARGET_DIR)
            })
            .or_else(|| {
                // Fallback: find a directory entry with `installdir` in its name
                directory_table
                    .keys()
                    .find(|name| name.to_ascii_uppercase().contains(INSTALL_DIR))
                    .and_then(|install_dir| {
                        Self::build_directory(directory_table, install_dir, TARGET_DIR)
                    })
            })
            .or_else(|| {
                // Fallback: get the first directory with zero or multiple subdirectories
                const SKIP_DIRECTORIES: [&str; 2] = ["DesktopFolder", "ProgramMenuFolder"];

                let mut path = Utf8PathBuf::new();
                let mut current_dir = TARGET_DIR;
                loop {
                    let sub_directories = directory_table
                        .iter()
                        .filter(|(directory, (directory_parent, _))| {
                            !SKIP_DIRECTORIES.contains(&directory.as_str())
                                && directory_parent.as_deref() == Some(current_dir)
                        })
                        .collect::<Vec<_>>();
                    if sub_directories.len() == 1 {
                        let (directory, (_directory_parent, default_dir)) = sub_directories[0];
                        current_dir = directory;
                        path.push(
                            Self::get_property_relative_path(current_dir).unwrap_or(default_dir),
                        );
                    } else {
                        break;
                    }
                }
                Option::from(path).filter(|path| !path.as_str().is_empty())
            })
    }

    /// Constructs a path from the root directory to the target subdirectory based on the directory
    /// table.
    ///
    /// This is deliberately recursive so that the function can start at the deepest directory,
    /// traverse upwards, and then build the path sequentially as the stack is unwinding. Using a
    /// loop would require the path components to be reversed at the end.
    ///
    /// [Using the Directory Table](https://learn.microsoft.com/windows/win32/msi/using-the-directory-table)
    fn build_directory(
        directory_table: &HashMap<String, (Option<String>, String)>,
        current_dir: &str,
        target_dir: &str,
    ) -> Option<Utf8PathBuf> {
        // If the current directory is the target, return an empty path
        if current_dir == target_dir {
            return Some(Utf8PathBuf::new());
        }

        if let Some((Some(parent), default_dir)) = directory_table.get(current_dir) {
            if let Some(mut path) = Self::build_directory(directory_table, parent, target_dir) {
                path.push(Self::get_property_relative_path(current_dir).unwrap_or(default_dir));
                return Some(path);
            }
        }

        None
    }

    fn get_property_relative_path(property: &str) -> Option<&str> {
        const PROGRAM_FILES_64_FOLDER: &str = "ProgramFiles64Folder";
        const PROGRAM_FILES_FOLDER: &str = "ProgramFilesFolder";
        const COMMON_FILES_64_FOLDER: &str = "CommonFiles64Folder";
        const COMMON_FILES_FOLDER: &str = "CommonFilesFolder";
        const APP_DATA_FOLDER: &str = "AppDataFolder";
        const LOCAL_APP_DATA_FOLDER: &str = "LocalAppDataFolder";
        const TEMP_FOLDER: &str = "TempFolder";
        const WINDOWS_FOLDER: &str = "WindowsFolder";

        match property {
            PROGRAM_FILES_64_FOLDER => Some(RELATIVE_PROGRAM_FILES_64),
            PROGRAM_FILES_FOLDER => Some(RELATIVE_PROGRAM_FILES_32),
            COMMON_FILES_64_FOLDER => Some(RELATIVE_COMMON_FILES_64),
            COMMON_FILES_FOLDER => Some(RELATIVE_COMMON_FILES_32),
            APP_DATA_FOLDER => Some(RELATIVE_APP_DATA),
            LOCAL_APP_DATA_FOLDER => Some(RELATIVE_LOCAL_APP_DATA),
            TEMP_FOLDER => Some(RELATIVE_TEMP_FOLDER),
            WINDOWS_FOLDER => Some(RELATIVE_WINDOWS_DIR),
            _ => None,
        }
    }

    /// Google Chrome translates its `ProductVersion` into a different one. The actual
    /// `DisplayVersion` can be retrieved from the MSI Summary Info Comments.
    ///
    /// <https://issues.chromium.org/issues/382215764#comment8>
    fn get_actual_chrome_version<R: Read + Seek>(msi: &Package<R>) -> Option<&str> {
        msi.summary_info()
            .comments()
            .map(str::split_ascii_whitespace)
            .as_mut()
            .and_then(SplitAsciiWhitespace::next)
            .filter(|version| version.split('.').all(|part| part.parse::<u16>().is_ok()))
    }
}
