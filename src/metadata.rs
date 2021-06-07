use crate::{CargoToml, PyProjectToml};
use anyhow::{bail, Context, Result};
use fs_err as fs;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str;

/// The metadata required to generate the .dist-info directory
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct WheelMetadata {
    /// Python Package Metadata 2.2
    pub metadata22: Metadata22,
    /// The `[console_scripts]` for the entry_points.txt
    pub scripts: HashMap<String, String>,
    /// The name of the module can be distinct from the package name, mostly
    /// because package names normally contain minuses while module names
    /// have underscores. The package name is part of metadata22
    pub module_name: String,
}

/// Python Package Metadata 2.2 as specified in
/// https://packaging.python.org/specifications/core-metadata/
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[allow(missing_docs)]
pub struct Metadata22 {
    // Mandatory fields
    pub metadata_version: String,
    pub name: String,
    pub version: String,
    // Optional fields
    pub platform: Vec<String>,
    pub supported_platform: Vec<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub description_content_type: Option<String>,
    pub keywords: Option<String>,
    pub home_page: Option<String>,
    pub download_url: Option<String>,
    pub author: Option<String>,
    pub author_email: Option<String>,
    pub maintainer: Option<String>,
    pub maintainer_email: Option<String>,
    pub license: Option<String>,
    pub classifiers: Vec<String>,
    pub requires_dist: Vec<String>,
    pub provides_dist: Vec<String>,
    pub obsoletes_dist: Vec<String>,
    pub requires_python: Option<String>,
    pub requires_external: Vec<String>,
    pub project_url: HashMap<String, String>,
    pub provides_extra: Vec<String>,
    pub scripts: HashMap<String, String>,
    pub gui_scripts: HashMap<String, String>,
    pub entry_points: HashMap<String, HashMap<String, String>>,
}

const PLAINTEXT_CONTENT_TYPE: &str = "text/plain; charset=UTF-8";
const GFM_CONTENT_TYPE: &str = "text/markdown; charset=UTF-8; variant=GFM";

/// Guess a Description-Content-Type based on the file extension,
/// defaulting to plaintext if extension is unknown or empty.
///
/// See https://packaging.python.org/specifications/core-metadata/#description-content-type
fn path_to_content_type(path: &Path) -> String {
    path.extension()
        .map_or(String::from(PLAINTEXT_CONTENT_TYPE), |ext| {
            let ext = ext.to_string_lossy().to_lowercase();
            let type_str = match ext.as_str() {
                "rst" => "text/x-rst; charset=UTF-8",
                "md" => GFM_CONTENT_TYPE,
                "markdown" => GFM_CONTENT_TYPE,
                _ => PLAINTEXT_CONTENT_TYPE,
            };
            String::from(type_str)
        })
}

impl Metadata22 {
    /// Merge metadata with pyproject.toml, where pyproject.toml takes precedence
    ///
    /// manifest_path must be the directory, not the file
    fn merge_pyproject_toml(&mut self, manifest_path: impl AsRef<Path>) -> Result<()> {
        let manifest_path = manifest_path.as_ref();
        if !manifest_path.join("pyproject.toml").is_file() {
            return Ok(());
        }
        let pyproject_toml =
            PyProjectToml::new(manifest_path).context("pyproject.toml is invalid")?;
        if let Some(project) = &pyproject_toml.project {
            self.name = project.name.clone();

            if let Some(version) = &project.version {
                self.version = version.clone();
            }

            if let Some(description) = &project.description {
                self.summary = Some(description.clone());
            }

            match &project.readme {
                Some(pyproject_toml::ReadMe::RelativePath(readme_path)) => {
                    let readme_path = manifest_path.join(readme_path);
                    let description = Some(fs::read_to_string(&readme_path).context(format!(
                        "Failed to read readme specified in pyproject.toml, which should be at {}",
                        readme_path.display()
                    ))?);
                    self.description = description;
                    self.description_content_type = Some(path_to_content_type(&readme_path));
                }
                Some(pyproject_toml::ReadMe::Table {
                    file,
                    text,
                    content_type,
                }) => {
                    if file.is_some() && text.is_some() {
                        bail!("file and text fields of 'project.readme' are mutually-exclusive, only one of them should be specified");
                    }
                    if let Some(readme_path) = file {
                        let readme_path = manifest_path.join(readme_path);
                        let description = Some(fs::read_to_string(&readme_path).context(format!(
                                "Failed to read readme specified in pyproject.toml, which should be at {}",
                                readme_path.display()
                            ))?);
                        self.description = description;
                    }
                    if let Some(description) = text {
                        self.description = Some(description.clone());
                    }
                    self.description_content_type = content_type.clone();
                }
                None => {}
            }

            if let Some(requires_python) = &project.requires_python {
                self.requires_python = Some(requires_python.clone());
            }

            if let Some(pyproject_toml::License { file, text }) = &project.license {
                if file.is_some() && text.is_some() {
                    bail!("file and text fields of 'project.license' are mutually-exclusive, only one of them should be specified");
                }
                if let Some(license_path) = file {
                    let license_path = manifest_path.join(license_path);
                    self.license = Some(fs::read_to_string(&license_path).context(format!(
                            "Failed to read license file specified in pyproject.toml, which should be at {}",
                            license_path.display()
                        ))?);
                }
                if let Some(license_text) = text {
                    self.license = Some(license_text.clone());
                }
            }

            if let Some(authors) = &project.authors {
                let mut names = Vec::with_capacity(authors.len());
                let mut emails = Vec::with_capacity(authors.len());
                for author in authors {
                    match (&author.name, &author.email) {
                        (Some(name), Some(email)) => {
                            emails.push(format!("{} <{}>", name, email));
                        }
                        (Some(name), None) => {
                            names.push(name.as_str());
                        }
                        (None, Some(email)) => {
                            emails.push(email.clone());
                        }
                        (None, None) => {}
                    }
                }
                self.author = Some(names.join(", "));
                self.author_email = Some(emails.join(", "));
            }

            if let Some(maintainers) = &project.maintainers {
                let mut names = Vec::with_capacity(maintainers.len());
                let mut emails = Vec::with_capacity(maintainers.len());
                for maintainer in maintainers {
                    match (&maintainer.name, &maintainer.email) {
                        (Some(name), Some(email)) => {
                            emails.push(format!("{} <{}>", name, email));
                        }
                        (Some(name), None) => {
                            names.push(name.as_str());
                        }
                        (None, Some(email)) => {
                            emails.push(email.clone());
                        }
                        (None, None) => {}
                    }
                }
                self.maintainer = Some(names.join(", "));
                self.maintainer_email = Some(emails.join(", "));
            }

            if let Some(keywords) = &project.keywords {
                self.keywords = Some(keywords.join(","));
            }

            if let Some(classifiers) = &project.classifiers {
                self.classifiers = classifiers.clone();
            }

            if let Some(urls) = &project.urls {
                self.project_url = urls.clone();
            }

            if let Some(dependencies) = &project.dependencies {
                self.requires_dist = dependencies.clone();
            }

            if let Some(dependencies) = &project.optional_dependencies {
                for (extra, deps) in dependencies {
                    self.provides_extra.push(extra.clone());
                    for dep in deps {
                        let dist = if let Some((dep, marker)) = dep.split_once(';') {
                            // optional dependency already has environment markers
                            let new_marker =
                                format!("({}) and extra == '{}'", marker.trim(), extra);
                            format!("{}; {}", dep, new_marker)
                        } else {
                            format!("{}; extra == '{}'", dep, extra)
                        };
                        self.requires_dist.push(dist);
                    }
                }
            }

            if let Some(scripts) = &project.scripts {
                self.scripts = scripts.clone();
            }
            if let Some(gui_scripts) = &project.gui_scripts {
                self.gui_scripts = gui_scripts.clone();
            }
            if let Some(entry_points) = &project.entry_points {
                // Raise error on ambiguous entry points: https://www.python.org/dev/peps/pep-0621/#entry-points
                if entry_points.contains_key("console_scripts") {
                    bail!("console_scripts is not allowed in project.entry-points table");
                }
                if entry_points.contains_key("gui_scripts") {
                    bail!("gui_scripts is not allowed in project.entry-points table");
                }
                self.entry_points = entry_points.clone();
            }
        }
        Ok(())
    }
    /// Uses a Cargo.toml to create the metadata for python packages
    ///
    /// manifest_path must be the directory, not the file
    pub fn from_cargo_toml(
        cargo_toml: &CargoToml,
        manifest_path: impl AsRef<Path>,
    ) -> Result<Metadata22> {
        let authors = cargo_toml.package.authors.join(", ");

        let classifiers = cargo_toml.classifiers();

        let author_email = if authors.contains('@') {
            Some(authors.clone())
        } else {
            None
        };

        let extra_metadata = cargo_toml.remaining_core_metadata();

        let description: Option<String>;
        let description_content_type: Option<String>;
        // See https://packaging.python.org/specifications/core-metadata/#description
        if let Some(ref readme) = cargo_toml.package.readme {
            let readme_path = manifest_path.as_ref().join(readme);
            description = Some(fs::read_to_string(&readme_path).context(format!(
                "Failed to read readme specified in Cargo.toml, which should be at {}",
                readme_path.display()
            ))?);

            description_content_type = extra_metadata
                .description_content_type
                .or_else(|| Some(path_to_content_type(&readme_path)));
        } else {
            description = None;
            description_content_type = None;
        };
        let name = extra_metadata
            .name
            .map(|name| {
                if let Some(pos) = name.find('.') {
                    name.split_at(pos).0.to_string()
                } else {
                    name.clone()
                }
            })
            .unwrap_or_else(|| cargo_toml.package.name.clone());

        let mut metadata = Metadata22 {
            metadata_version: "2.2".to_owned(),

            // Mapped from cargo metadata
            name,
            version: cargo_toml.package.version.clone(),
            summary: cargo_toml.package.description.clone(),
            description,
            description_content_type,
            keywords: cargo_toml
                .package
                .keywords
                .clone()
                .map(|keywords| keywords.join(",")),
            home_page: cargo_toml.package.homepage.clone(),
            download_url: None,
            // Cargo.toml has no distinction between author and author email
            author: Some(authors),
            author_email,
            license: cargo_toml.package.license.clone(),

            // Values provided through `[project.metadata.maturin]`
            classifiers,
            maintainer: extra_metadata.maintainer,
            maintainer_email: extra_metadata.maintainer_email,
            requires_dist: extra_metadata.requires_dist.unwrap_or_default(),
            requires_python: extra_metadata.requires_python,
            requires_external: extra_metadata.requires_external.unwrap_or_default(),
            project_url: extra_metadata.project_url.unwrap_or_default(),
            provides_extra: extra_metadata.provides_extra.unwrap_or_default(),

            // Officially rarely used, and afaik not applicable with pyo3
            provides_dist: Vec::new(),
            obsoletes_dist: Vec::new(),

            // Open question: Should those also be supported? And if so, how?
            platform: Vec::new(),
            supported_platform: Vec::new(),
            scripts: cargo_toml.scripts(),
            gui_scripts: HashMap::new(),
            entry_points: HashMap::new(),
        };
        metadata.merge_pyproject_toml(manifest_path)?;
        Ok(metadata)
    }

    /// Formats the metadata into a list where keys with multiple values
    /// become multiple single-valued key-value pairs. This format is needed for the pypi
    /// uploader and for the METADATA file inside wheels
    pub fn to_vec(&self) -> Vec<(String, String)> {
        let mut fields = vec![
            ("Metadata-Version", self.metadata_version.clone()),
            ("Name", self.name.clone()),
            ("Version", self.get_version_escaped()),
        ];

        let mut add_vec = |name, values: &[String]| {
            for i in values {
                fields.push((name, i.clone()));
            }
        };

        add_vec("Platform", &self.platform);
        add_vec("Supported-Platform", &self.supported_platform);
        add_vec("Classifier", &self.classifiers);
        add_vec("Requires-Dist", &self.requires_dist);
        add_vec("Provides-Dist", &self.provides_dist);
        add_vec("Obsoletes-Dist", &self.obsoletes_dist);
        add_vec("Requires-External", &self.requires_external);
        add_vec("Provides-Extra", &self.provides_extra);

        let mut add_option = |name, value: &Option<String>| {
            if let Some(some) = value.clone() {
                fields.push((name, some));
            }
        };

        add_option("Summary", &self.summary);
        add_option("Keywords", &self.keywords);
        add_option("Home-Page", &self.home_page);
        add_option("Download-URL", &self.download_url);
        add_option("Author", &self.author);
        add_option("Author-email", &self.author_email);
        add_option("Maintainer", &self.maintainer);
        add_option("Maintainer-email", &self.maintainer_email);
        add_option("License", &self.license);
        add_option("Requires-Python", &self.requires_python);
        add_option("Description-Content-Type", &self.description_content_type);
        // Project-URL is special
        // "A string containing a browsable URL for the project and a label for it, separated by a comma."
        // `Project-URL: Bug Tracker, http://bitbucket.org/tarek/distribute/issues/`
        for (key, value) in self.project_url.iter() {
            fields.push(("Project-URL", format!("{}, {}", key, value)))
        }

        // Description shall be last, so we can ignore RFC822 and just put the description
        // in the body
        // The borrow checker doesn't like us using add_option here
        if let Some(description) = &self.description {
            fields.push(("Description", description.clone()));
        }

        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    /// Writes the format for the metadata file inside wheels
    pub fn to_file_contents(&self) -> String {
        let mut fields = self.to_vec();
        let mut out = "".to_string();
        let body = match fields.last() {
            Some((key, description)) if key == "Description" => {
                let desc = description.clone();
                fields.pop().unwrap();
                Some(desc)
            }
            Some((_, _)) => None,
            None => None,
        };

        for (key, value) in fields {
            out += &format!("{}: {}\n", key, value);
        }

        if let Some(body) = body {
            out += &format!("\n{}\n", body);
        }

        out
    }

    /// Returns the distribution name according to PEP 427, Section "Escaping
    /// and Unicode"
    pub fn get_distribution_escaped(&self) -> String {
        let re = Regex::new(r"[^\w\d.]+").unwrap();
        re.replace_all(&self.name, "_").to_string()
    }

    /// Returns the version encoded according to PEP 427, Section "Escaping
    /// and Unicode"
    pub fn get_version_escaped(&self) -> String {
        let re = Regex::new(r"[^\w\d.]+").unwrap();
        re.replace_all(&self.version, "_").to_string()
    }

    /// Returns the name of the .dist-info directory as defined in the wheel specification
    pub fn get_dist_info_dir(&self) -> PathBuf {
        PathBuf::from(format!(
            "{}-{}.dist-info",
            &self.get_distribution_escaped(),
            &self.get_version_escaped()
        ))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;
    use std::io::Write;

    fn assert_metadata_from_cargo_toml(readme: &str, cargo_toml: &str, expected: &str) {
        let mut readme_md = tempfile::NamedTempFile::new().unwrap();

        let readme_path = if cfg!(windows) {
            readme_md.path().to_str().unwrap().replace("\\", "/")
        } else {
            readme_md.path().to_str().unwrap().to_string()
        };

        readme_md.write_all(readme.as_bytes()).unwrap();

        let toml_with_path = cargo_toml.replace("REPLACE_README_PATH", &readme_path);

        let cargo_toml_struct: CargoToml = toml::from_str(&toml_with_path).unwrap();

        let metadata =
            Metadata22::from_cargo_toml(&cargo_toml_struct, &readme_md.path().parent().unwrap())
                .unwrap();

        let actual = metadata.to_file_contents();

        assert_eq!(
            actual.trim(),
            expected.trim(),
            "Actual metadata differed from expected\nEXPECTED:\n{}\n\nGOT:\n{}",
            expected,
            actual
        );

        // get_dist_info_dir test checks against hard-coded values - check that they are as expected in the source first
        assert!(
            cargo_toml.contains("name = \"info-project\"")
                && cargo_toml.contains("version = \"0.1.0\""),
            "cargo_toml name and version string do not match hardcoded values, test will fail",
        );
        assert_eq!(
            metadata.get_dist_info_dir(),
            PathBuf::from("info_project-0.1.0.dist-info"),
            "Dist info dir differed from expected"
        );
    }

    #[test]
    fn test_metadata_from_cargo_toml() {
        let readme = indoc!(
            r#"
            # Some test package

            This is the readme for a test package
        "#
        );

        let cargo_toml = indoc!(
            r#"
            [package]
            authors = ["konstin <konstin@mailbox.org>"]
            name = "info-project"
            version = "0.1.0"
            description = "A test project"
            homepage = "https://example.org"
            readme = "REPLACE_README_PATH"
            keywords = ["ffi", "test"]

            [lib]
            crate-type = ["cdylib"]
            name = "pyo3_pure"

            [package.metadata.maturin.scripts]
            ph = "maturin:print_hello"

            [package.metadata.maturin]
            classifiers = ["Programming Language :: Python"]
            requires-dist = ["flask~=1.1.0", "toml==0.10.0"]
            project-url = { "Bug Tracker" = "http://bitbucket.org/tarek/distribute/issues/" }
        "#
        );

        let expected = indoc!(
            r#"
            Metadata-Version: 2.2
            Name: info-project
            Version: 0.1.0
            Classifier: Programming Language :: Python
            Requires-Dist: flask~=1.1.0
            Requires-Dist: toml==0.10.0
            Summary: A test project
            Keywords: ffi,test
            Home-Page: https://example.org
            Author: konstin <konstin@mailbox.org>
            Author-email: konstin <konstin@mailbox.org>
            Description-Content-Type: text/plain; charset=UTF-8
            Project-URL: Bug Tracker, http://bitbucket.org/tarek/distribute/issues/

            # Some test package

            This is the readme for a test package
        "#
        );

        assert_metadata_from_cargo_toml(readme, cargo_toml, expected);
    }

    #[test]
    fn test_metadata_from_cargo_toml_rst() {
        let readme = indoc!(
            r#"
            Some test package
            =================
        "#
        );

        let cargo_toml = indoc!(
            r#"
            [package]
            authors = ["konstin <konstin@mailbox.org>"]
            name = "info-project"
            version = "0.1.0"
            description = "A test project"
            homepage = "https://example.org"
            readme = "REPLACE_README_PATH"
            keywords = ["ffi", "test"]

            [lib]
            crate-type = ["cdylib"]
            name = "pyo3_pure"

            [package.metadata.maturin.scripts]
            ph = "maturin:print_hello"

            [package.metadata.maturin]
            classifiers = ["Programming Language :: Python"]
            requires-dist = ["flask~=1.1.0", "toml==0.10.0"]
            description-content-type = "text/x-rst"
        "#
        );

        let expected = indoc!(
            r#"
            Metadata-Version: 2.2
            Name: info-project
            Version: 0.1.0
            Classifier: Programming Language :: Python
            Requires-Dist: flask~=1.1.0
            Requires-Dist: toml==0.10.0
            Summary: A test project
            Keywords: ffi,test
            Home-Page: https://example.org
            Author: konstin <konstin@mailbox.org>
            Author-email: konstin <konstin@mailbox.org>
            Description-Content-Type: text/x-rst

            Some test package
            =================
        "#
        );

        assert_metadata_from_cargo_toml(readme, cargo_toml, expected);
    }

    #[test]
    fn test_metadata_from_cargo_toml_name_override() {
        let cargo_toml = indoc!(
            r#"
            [package]
            authors = ["konstin <konstin@mailbox.org>"]
            name = "info-project"
            version = "0.1.0"
            description = "A test project"
            homepage = "https://example.org"

            [lib]
            crate-type = ["cdylib"]
            name = "pyo3_pure"

            [package.metadata.maturin.scripts]
            ph = "maturin:print_hello"

            [package.metadata.maturin]
            name = "info"
            classifiers = ["Programming Language :: Python"]
            description-content-type = "text/x-rst"
        "#
        );

        let expected = indoc!(
            r#"
            Metadata-Version: 2.2
            Name: info
            Version: 0.1.0
            Classifier: Programming Language :: Python
            Summary: A test project
            Home-Page: https://example.org
            Author: konstin <konstin@mailbox.org>
            Author-email: konstin <konstin@mailbox.org>
        "#
        );

        let cargo_toml_struct: CargoToml = toml::from_str(&cargo_toml).unwrap();
        let metadata =
            Metadata22::from_cargo_toml(&cargo_toml_struct, "/not/exist/manifest/path").unwrap();
        let actual = metadata.to_file_contents();

        assert_eq!(
            actual.trim(),
            expected.trim(),
            "Actual metadata differed from expected\nEXPECTED:\n{}\n\nGOT:\n{}",
            expected,
            actual
        );

        assert_eq!(
            metadata.get_dist_info_dir(),
            PathBuf::from("info-0.1.0.dist-info"),
            "Dist info dir differed from expected"
        );
    }

    #[test]
    fn test_path_to_content_type() {
        for (filename, expected) in &[
            ("r.md", GFM_CONTENT_TYPE),
            ("r.markdown", GFM_CONTENT_TYPE),
            ("r.mArKdOwN", GFM_CONTENT_TYPE),
            ("r.rst", "text/x-rst; charset=UTF-8"),
            ("r.somethingelse", PLAINTEXT_CONTENT_TYPE),
            ("r", PLAINTEXT_CONTENT_TYPE),
        ] {
            let result = path_to_content_type(&PathBuf::from(filename));
            assert_eq!(
                &result.as_str(),
                expected,
                "Wrong content type for file '{}'. Expected '{}', got '{}'",
                filename,
                expected,
                result
            );
        }
    }

    #[test]
    fn test_merge_metadata_from_pyproject_toml() {
        let cargo_toml_str = fs_err::read_to_string("test-crates/pyo3-pure/Cargo.toml").unwrap();
        let cargo_toml: CargoToml = toml::from_str(&cargo_toml_str).unwrap();
        let metadata = Metadata22::from_cargo_toml(&cargo_toml, "test-crates/pyo3-pure").unwrap();
        assert_eq!(
            metadata.summary,
            Some("Implements a dummy function in Rust".to_string())
        );
        assert_eq!(
            metadata.description,
            Some(fs_err::read_to_string("test-crates/pyo3-pure/Readme.md").unwrap())
        );
        assert_eq!(metadata.classifiers, &["Programming Language :: Rust"]);
        assert_eq!(
            metadata.maintainer_email,
            Some("messense <messense@icloud.com>".to_string())
        );
        assert_eq!(metadata.scripts["get_42"], "pyo3_pure:DummyClass.get_42");
        assert_eq!(
            metadata.gui_scripts["get_42_gui"],
            "pyo3_pure:DummyClass.get_42"
        );
        assert_eq!(metadata.provides_extra, &["test"]);
        assert_eq!(
            metadata.requires_dist,
            &[
                "attrs; extra == 'test'",
                "boltons; (sys_platform == 'win32') and extra == 'test'"
            ]
        )
    }
}
