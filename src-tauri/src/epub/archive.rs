//! Opening an EPUB archive: locating, reading and parsing its OPF, and
//! reading entries with size caps (a crafted ZIP can claim a tiny entry that
//! inflates to gigabytes).

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use zip::ZipArchive;

use super::container::{parent_dir, rootfiles, CONTAINER_PATH};
use super::cover::{find_cover, Cover};
use super::metadata::{self, EpubMetadata};
use super::opf::Package;
use super::xml::XmlText;

/// Largest `container.xml` read.
const MAX_CONTAINER_BYTES: u64 = 1 << 20;
/// Largest OPF read (real ones are a few KB; huge manifests a few hundred).
pub(crate) const MAX_OPF_BYTES: u64 = 16 << 20;
/// Largest cover image read (the same cap as the other cover extractors in
/// `commands::library::covers`).
pub(crate) const MAX_IMAGE_BYTES: u64 = 50 << 20;

/// An EPUB's package document.
#[derive(Debug, Clone)]
pub(crate) struct Opf {
    /// The OPF's entry name in the archive, as stored (`OEBPS/content.opf`).
    pub path: String,
    /// Its directory in the archive (`OEBPS`), which manifest hrefs are
    /// relative to.
    pub dir: String,
    pub xml: XmlText,
    pub package: Package,
}

impl Opf {
    pub(crate) fn text(&self) -> &str {
        &self.xml.text
    }
}

/// How strictly to locate the OPF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Locate {
    /// Only through `META-INF/container.xml`, and the OPF must be editable
    /// (UTF-8). Used before modifying a book.
    Strict,
    /// Also accept a broken or missing `container.xml` when the archive has
    /// an `.opf` entry, and non-UTF-8 OPFs. Used for reading.
    Lenient,
}

/// An open EPUB: its archive and parsed OPF.
pub(crate) struct EpubArchive<R> {
    zip: ZipArchive<R>,
    opf: Opf,
}

impl EpubArchive<BufReader<File>> {
    /// Open the EPUB at `path` for reading.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
        Self::new(BufReader::new(file), Locate::Lenient).map_err(|e| {
            format!(
                "{}: {e}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            )
        })
    }
}

impl<R: Read + Seek> EpubArchive<R> {
    pub(crate) fn new(reader: R, locate: Locate) -> Result<Self, String> {
        let mut zip =
            ZipArchive::new(reader).map_err(|e| format!("not a valid EPUB (ZIP) file: {e}"))?;
        let opf = locate_opf(&mut zip, locate)?;
        Ok(Self { zip, opf })
    }

    pub(crate) fn opf(&self) -> &Opf {
        &self.opf
    }

    pub(crate) fn zip(&mut self) -> &mut ZipArchive<R> {
        &mut self.zip
    }

    /// The book's metadata as the OPF states it.
    pub(crate) fn metadata(&self) -> EpubMetadata {
        metadata::read(&self.opf.package)
    }

    /// The stored name of the entry at `path`: an exact match, else a
    /// case-insensitive one (archives made on case-insensitive disks).
    pub(crate) fn entry_name(&self, path: &str) -> Option<String> {
        entry_name(&self.zip, path)
    }

    /// The cover to show ([`find_cover`]) and its bytes. `None` when there is
    /// none, its entry is missing or empty, or it is too large.
    pub(crate) fn cover_image(&mut self) -> Option<(Cover, Vec<u8>)> {
        let cover = find_cover(&self.opf.package, &self.opf.dir)?;
        let name = self.entry_name(&cover.path)?;
        let bytes = read_entry(&mut self.zip, &name, MAX_IMAGE_BYTES).ok()?;
        (!bytes.is_empty()).then_some((cover, bytes))
    }
}

/// See [`EpubArchive::entry_name`].
pub(crate) fn entry_name<R: Read + Seek>(zip: &ZipArchive<R>, path: &str) -> Option<String> {
    if zip.index_for_name(path).is_some() {
        return Some(path.to_string());
    }
    // (zip 9 reports names it can't decode as errors; they can't match.)
    zip.file_names()
        .filter_map(Result::ok)
        .find(|n| n.eq_ignore_ascii_case(path))
        .map(|n| n.into_owned())
}

/// Read the entry `name`, refusing more than `limit` bytes (checked while
/// inflating, not trusting the size the archive claims).
pub(crate) fn read_entry<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let entry = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    if entry.size() > limit {
        return Err(format!("{name} is too large ({} bytes)", entry.size()));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Could not read {name}: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{name} is too large"));
    }
    Ok(bytes)
}

/// Find, read and parse the OPF of an archive (see [`Locate`]).
pub(crate) fn locate_opf<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    locate: Locate,
) -> Result<Opf, String> {
    let from_container = (|| -> Result<String, String> {
        let name = entry_name(zip, CONTAINER_PATH)
            .ok_or_else(|| format!("{CONTAINER_PATH} is missing"))?;
        let container = XmlText::decode(&read_entry(zip, &name, MAX_CONTAINER_BYTES)?);
        let candidates = rootfiles(&container.text)?;
        candidates
            .iter()
            .find_map(|path| entry_name(zip, path))
            .ok_or_else(|| format!("{CONTAINER_PATH} names no OPF that exists in the archive"))
    })();
    let path = match (from_container, locate) {
        (Ok(path), _) => path,
        (Err(e), Locate::Strict) => return Err(e),
        (Err(e), Locate::Lenient) => {
            let mut opfs = zip
                .file_names()
                .filter_map(Result::ok)
                .filter(|n| n.to_ascii_lowercase().ends_with(".opf"));
            opfs.next().map(|n| n.into_owned()).ok_or(e)?
        }
    };
    let xml = XmlText::decode(&read_entry(zip, &path, MAX_OPF_BYTES)?);
    if locate == Locate::Strict && !xml.editable {
        return Err(format!(
            "{path} is not UTF-8 encoded; Librarium only edits UTF-8 OPF files"
        ));
    }
    let package = Package::parse(&xml.text).map_err(|e| format!("{path}: {e}"))?;
    let dir = parent_dir(&path).to_string();
    Ok(Opf {
        path,
        dir,
        xml,
        package,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::epub::fixtures::{container, epub_bytes, CALIBRE_OPF, EPUB3_OPF, PNG};

    fn open(bytes: Vec<u8>, locate: Locate) -> Result<EpubArchive<Cursor<Vec<u8>>>, String> {
        EpubArchive::new(Cursor::new(bytes), locate)
    }

    #[test]
    fn the_opf_is_found_through_the_container_whatever_its_case() {
        let container = container("oebps/Package.OPF");
        let bytes = epub_bytes(&[
            ("META-INF/container.xml", container.as_bytes()),
            ("OEBPS/package.opf", EPUB3_OPF.as_bytes()),
            ("OEBPS/images/cover.png", PNG),
        ]);
        let mut epub = open(bytes, Locate::Strict).unwrap();
        assert_eq!(
            epub.opf().path,
            "OEBPS/package.opf",
            "the stored spelling is used"
        );
        assert_eq!(epub.opf().dir, "OEBPS");
        assert_eq!(epub.metadata().title.as_deref(), Some("Foundation"));
        let (cover, bytes) = epub.cover_image().unwrap();
        assert_eq!(
            (cover.path.as_str(), bytes.as_slice()),
            ("OEBPS/images/cover.png", PNG)
        );
    }

    #[test]
    fn a_broken_container_is_tolerated_only_for_reading() {
        let bytes = epub_bytes(&[
            ("META-INF/container.xml", b"<container><rootfiles>"),
            ("content.opf", CALIBRE_OPF.as_bytes()),
        ]);
        assert!(open(bytes.clone(), Locate::Strict).is_err());
        assert_eq!(
            open(bytes, Locate::Lenient).unwrap().opf().path,
            "content.opf"
        );

        let no_opf = epub_bytes(&[(
            "META-INF/container.xml",
            container("content.opf").as_bytes(),
        )]);
        assert!(open(no_opf, Locate::Lenient).is_err());
        assert!(open(b"not a zip".to_vec(), Locate::Lenient).is_err());
    }

    #[test]
    fn non_utf8_opfs_are_read_but_not_edited() {
        let latin1: Vec<u8> = CALIBRE_OPF
            .replace("Dune</dc:title>", "Dun\u{e9}</dc:title>")
            .chars()
            .map(|c| c as u8)
            .collect();
        let bytes = epub_bytes(&[
            (
                "META-INF/container.xml",
                container("content.opf").as_bytes(),
            ),
            ("content.opf", &latin1),
        ]);
        assert_eq!(
            open(bytes.clone(), Locate::Lenient)
                .unwrap()
                .metadata()
                .title
                .as_deref(),
            Some("Duné")
        );
        let err = open(bytes, Locate::Strict).err().unwrap();
        assert!(err.contains("not UTF-8"), "{err}");
    }

    #[test]
    fn entries_are_read_with_a_size_cap() {
        let big = vec![b'a'; 4096];
        let bytes = epub_bytes(&[("big.txt", &big)]);
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(read_entry(&mut zip, "big.txt", 4096).unwrap().len(), 4096);
        assert!(read_entry(&mut zip, "big.txt", 4095).is_err());
        assert!(read_entry(&mut zip, "missing.txt", 10).is_err());
    }
}
