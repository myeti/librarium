//! The library book id *inside* the files Librarium sends, so a device copy
//! can be traced back to its library book even without the manifest
//! (lost, or written by Librarium on another computer):
//!
//! - **AZW3** (converted for a Kindle): the id is the book's ASIN, EXTH
//!   record 113 (boko's `Azw3Config::asin`, with cdetype `EBOK` in EXTH
//!   501), as Calibre does with its own UUIDs. Kindle names home-screen
//!   thumbnails after ASIN + cdetype, so each book has its own, not boko's
//!   shared `EBOK000000` placeholder.
//! - **EPUB**: an extra `<dc:identifier>urn:librarium:<id></dc:identifier>`
//!   ([`crate::epub::identity`]), added to the copy only.
//!
//! Books sent in their own format (an AZW3, MOBI or PDF from the library)
//! are not modified: only the manifest knows them.
//!
//! The same header gives a MOBI/AZW3's title and authors for the device
//! listing ([`kindle_details`]): EXTH 503 (the title) or the MOBI header's
//! full name, and EXTH 100 (one record per author), in the file's text
//! encoding (UTF-8, or Windows-1252 in older MOBI files).
//!
//! Checked on a Kindle Paperwhite 3 (2026-11-09): the books are accepted.
//! The Kindle doesn't make the home-screen cover of a book with an ASIN
//! itself, so Librarium writes it (`thumbnail.rs`, checked on that Kindle
//! on 2026-11-10).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::epub::{identity as epub_identity, EpubArchive};

/// Largest prefix of a MOBI/AZW3 file read to find its EXTH header: the PDB
/// header, the record table and record 0 (MOBI header + EXTH + title, which
/// boko pads with 8 KB). Calibre's and Amazon's record 0 are smaller still.
const MAX_RECORD0_READ: u64 = 512 * 1024;

/// What is read first: the 78-byte PDB header and the first two entries of
/// the record table (8 bytes each), enough to know where record 0 ends.
const PDB_HEAD: usize = 78 + 2 * 8;

/// Read when the record table can't tell where record 0 ends.
const FALLBACK_READ: u64 = 64 * 1024;

/// EXTH record type of the ASIN.
const EXTH_ASIN: u32 = 113;

/// EXTH record types of an author (one record per author) and of the title.
const EXTH_AUTHOR: u32 = 100;
const EXTH_TITLE: u32 = 503;

/// In record 0 (from its start): the text encoding, and where the full name
/// (the title) is and how long it is.
const MOBI_ENCODING: usize = 0x1C;
const MOBI_FULL_NAME_OFFSET: usize = 0x54;
const MOBI_FULL_NAME_LENGTH: usize = 0x58;

/// The text encodings a MOBI header declares.
const ENCODING_CP1252: u32 = 1252;

/// Title, author(s) (joined with " & ") and the embedded library id of an
/// EPUB on the device, from one read of its OPF. `None` if it can't be read
/// (a malformed file, even one that panics the parser, is treated like one
/// without metadata).
pub(crate) fn epub_details(path: &Path) -> Option<(String, String, Option<String>)> {
    std::panic::catch_unwind(|| {
        let epub = EpubArchive::open(path).ok()?;
        let meta = epub.metadata();
        let id = epub_identity::library_id(&epub.opf().package);
        Some((meta.title.unwrap_or_default(), meta.authors.join(" & "), id))
    })
    .ok()
    .flatten()
}

/// EXTH record type of the document type ("cdetype": `EBOK`, `PDOC`…).
const EXTH_CDETYPE: u32 = 501;

/// What the device listing shows of a MOBI/AZW3 file: its title, its
/// authors (joined with " & ") and the library id it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KindleDetails {
    pub title: String,
    pub author: String,
    pub library_id: Option<String>,
}

/// The title, authors and library id of the MOBI/AZW3 file at `path`, from
/// one read of its header ([`read_record0`]). The library id is the ASIN
/// (EXTH 113) when it is a UUID, which is what Librarium writes: Amazon
/// ASINs (`B00…`), boko's `EBOK000000` placeholder and other values are not
/// library ids (Calibre also writes UUIDs; one that isn't a library book's
/// id simply matches no library book). `None` when it isn't a MOBI/AZW3
/// file; empty fields when it says nothing.
pub(crate) fn kindle_details(path: &Path) -> Option<KindleDetails> {
    kindle_details_of(&read_record0(path)?)
}

/// [`kindle_details`] of a file whose first bytes are `data`.
pub(crate) fn kindle_details_of(data: &[u8]) -> Option<KindleDetails> {
    let header = MobiHeader::parse(data)?;
    let title = header
        .exth(EXTH_TITLE)
        .next()
        .filter(|t| !t.is_empty())
        .or_else(|| header.full_name())
        .unwrap_or_default();
    let mut authors: Vec<String> = Vec::new();
    for author in header.exth(EXTH_AUTHOR) {
        if !author.is_empty() && !authors.contains(&author) {
            authors.push(author);
        }
    }
    let library_id = header
        .exth(EXTH_ASIN)
        .next()
        .filter(|asin| uuid::Uuid::parse_str(asin).is_ok());
    Some(KindleDetails {
        title,
        author: authors.join(" & "),
        library_id,
    })
}

/// The ASIN (EXTH 113) and document type (EXTH 501) of the MOBI/AZW3 file at
/// `path`, read from one pass over its header. These name the book's
/// home-screen thumbnail on a Kindle (see `thumbnail.rs`).
pub(crate) fn kindle_ids(path: &Path) -> Option<(String, String)> {
    let head = read_record0(path)?;
    Some((
        exth_string(&head, EXTH_ASIN)?,
        exth_string(&head, EXTH_CDETYPE)?,
    ))
}

/// The first bytes of a MOBI/AZW3 file, up to the end of record 0: the PDB
/// header first, then up to the start of record 1 (from the record table).
/// On a real Kindle record 0 ends within ~20 KB, so listing 300 books reads
/// a few MB over USB instead of 150 MB (which made the device's books take
/// a long time to show).
fn read_record0(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut head = vec![0; PDB_HEAD];
    file.read_exact(&mut head).ok()?;
    let end = record0_end(&head)
        .unwrap_or(FALLBACK_READ)
        .min(MAX_RECORD0_READ);
    if end > head.len() as u64 {
        file.take(end - head.len() as u64)
            .read_to_end(&mut head)
            .ok()?;
    }
    Some(head)
}

/// Where record 0 of a PDB file ends (the offset of record 1), from the
/// first [`PDB_HEAD`] bytes. `None` when the record table doesn't say.
fn record0_end(head: &[u8]) -> Option<u64> {
    let count = u16::from_be_bytes(head.get(76..78)?.try_into().ok()?);
    if count < 2 {
        return None;
    }
    let record1 = u32::from_be_bytes(head.get(86..90)?.try_into().ok()?) as u64;
    (record1 > PDB_HEAD as u64).then_some(record1)
}

/// The EXTH record `wanted` of a MOBI/AZW3 file whose first bytes are
/// `data`, as text (the first, when there are several). A truncated or
/// malformed file gives `None`, never a panic.
pub(crate) fn exth_string(data: &[u8], wanted: u32) -> Option<String> {
    MobiHeader::parse(data)?
        .exth(wanted)
        .next()
        .filter(|text| !text.is_empty())
}

/// Record 0 of a MOBI/AZW3 file, located in its first bytes.
///
/// Layout: a 78-byte PDB header (type/creator `BOOKMOBI` at 60, record
/// count at 76) and its record table (8 bytes per record, the offset
/// first); record 0 starts with the 16-byte PalmDOC header, then the MOBI
/// header (`MOBI`, its length at +4, the text encoding at record 0 + 0x1C,
/// the full name's offset and length at + 0x54 and + 0x58, EXTH flag 0x40
/// at + 0x80), then the EXTH block (`EXTH`, length, count, then `type,
/// length, data` records, lengths including their 8-byte header). Every
/// offset is bounds-checked.
struct MobiHeader<'a> {
    data: &'a [u8],
    record0: usize,
    utf8: bool,
    /// Where the EXTH records start, and how many there are (none: no EXTH).
    exth: Option<(usize, u32)>,
}

impl<'a> MobiHeader<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        if data.get(60..68)? != b"BOOKMOBI" {
            return None;
        }
        let record0 = u32_at(data, 78)? as usize;
        if data.get(record0.checked_add(16)?..record0.checked_add(20)?)? != b"MOBI" {
            return None;
        }
        let utf8 = u32_at(data, record0 + MOBI_ENCODING)? != ENCODING_CP1252;
        let mobi_len = u32_at(data, record0 + 20)? as usize;
        let exth = (|| {
            if u32_at(data, record0 + 0x80)? & 0x40 == 0 {
                return None;
            }
            let start = record0.checked_add(16)?.checked_add(mobi_len)?;
            if data.get(start..start.checked_add(4)?)? != b"EXTH" {
                return None;
            }
            Some((start + 12, u32_at(data, start + 8)?))
        })();
        Some(Self {
            data,
            record0,
            utf8,
            exth,
        })
    }

    /// The text of every EXTH record of type `wanted`, in order (stops at
    /// the first malformed record).
    fn exth(&self, wanted: u32) -> impl Iterator<Item = String> + '_ {
        let (mut at, count) = self.exth.unwrap_or((0, 0));
        let mut left = count;
        std::iter::from_fn(move || {
            while left > 0 {
                left -= 1;
                let kind = u32_at(self.data, at)?;
                let len = u32_at(self.data, at + 4)? as usize;
                if len < 8 {
                    return None;
                }
                let value = self.data.get(at + 8..at.checked_add(len)?)?;
                at += len;
                if kind == wanted {
                    return Some(self.text(value));
                }
            }
            None
        })
    }

    /// The MOBI header's full name (the book's title), if it is in reach.
    fn full_name(&self) -> Option<String> {
        let offset = u32_at(self.data, self.record0 + MOBI_FULL_NAME_OFFSET)? as usize;
        let len = u32_at(self.data, self.record0 + MOBI_FULL_NAME_LENGTH)? as usize;
        let start = self.record0.checked_add(offset)?;
        let name = self.text(self.data.get(start..start.checked_add(len)?)?);
        (!name.is_empty()).then_some(name)
    }

    /// `bytes` decoded in the file's encoding, without padding and spaces.
    fn text(&self, bytes: &[u8]) -> String {
        let text = if self.utf8 {
            String::from_utf8_lossy(bytes).into_owned()
        } else {
            decode_cp1252(bytes)
        };
        text.trim_matches(char::from(0)).trim().to_string()
    }
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Windows-1252 (older MOBI files): Latin-1, except 0x80–0x9F, which hold
/// curly quotes, dashes, € and a few letters (Œ œ Š š Ž ž Ÿ).
fn decode_cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::device::tests::minimal_epub;
    use crate::convert::{convert_to_temp, convert_to_temp_with, ConvertOptions, OutputFormat};

    const ID: &str = "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11";

    #[test]
    fn an_azw3_converted_with_an_asin_carries_it_in_exth_113() {
        let dir = tempfile::tempdir().unwrap();
        let epub = dir.path().join("Book.epub");
        minimal_epub(&epub);

        let with_id = convert_to_temp_with(
            &epub,
            OutputFormat::Azw3,
            dir.path(),
            ConvertOptions {
                asin: Some(ID),
                ..Default::default()
            },
        )
        .unwrap();
        let head = std::fs::read(with_id.path()).unwrap();
        assert_eq!(exth_string(&head, EXTH_ASIN).as_deref(), Some(ID));
        // cdetype stays EBOK (EXTH 501): Kindle files the book as a book.
        assert_eq!(exth_string(&head, 501).as_deref(), Some("EBOK"));
        // The listing reads it all from the header: the id, the title, the author.
        assert_eq!(
            kindle_details(with_id.path()),
            Some(KindleDetails {
                title: "Test Book".into(),
                author: "Tester".into(),
                library_id: Some(ID.into()),
            })
        );

        let plain = convert_to_temp(&epub, OutputFormat::Azw3, dir.path()).unwrap();
        let head = std::fs::read(plain.path()).unwrap();
        assert_eq!(
            exth_string(&head, EXTH_ASIN).as_deref(),
            Some("EBOK000000"),
            "the placeholder without an ASIN"
        );
        assert_eq!(
            kindle_details(plain.path()).unwrap().library_id,
            None,
            "the placeholder isn't a library id"
        );
    }

    /// A MOBI header as old Kindle files have it: Windows-1252 text, the
    /// title only in the full name (no EXTH 503), one EXTH 100 per author.
    fn old_mobi(full_name: &[u8], records: &[(u32, &[u8])]) -> Vec<u8> {
        let record0 = 100usize;
        let mobi_len = 0xE8usize;
        let exth = record0 + 16 + mobi_len;
        let mut data = vec![0u8; exth];
        data[60..68].copy_from_slice(b"BOOKMOBI");
        data[78..82].copy_from_slice(&(record0 as u32).to_be_bytes());
        data[record0 + 16..record0 + 20].copy_from_slice(b"MOBI");
        data[record0 + 20..record0 + 24].copy_from_slice(&(mobi_len as u32).to_be_bytes());
        data[record0 + 0x1C..record0 + 0x20].copy_from_slice(&1252u32.to_be_bytes());
        data[record0 + 0x80..record0 + 0x84].copy_from_slice(&0x40u32.to_be_bytes());
        let mut block = Vec::new();
        for (kind, value) in records {
            block.extend_from_slice(&kind.to_be_bytes());
            block.extend_from_slice(&(value.len() as u32 + 8).to_be_bytes());
            block.extend_from_slice(value);
        }
        data.extend_from_slice(b"EXTH");
        data.extend_from_slice(&(block.len() as u32 + 12).to_be_bytes());
        data.extend_from_slice(&(records.len() as u32).to_be_bytes());
        data.extend_from_slice(&block);
        let name_at = data.len() - record0;
        data.extend_from_slice(full_name);
        data[record0 + 0x54..record0 + 0x58].copy_from_slice(&(name_at as u32).to_be_bytes());
        data[record0 + 0x58..record0 + 0x5C]
            .copy_from_slice(&(full_name.len() as u32).to_be_bytes());
        data
    }

    #[test]
    fn an_old_mobi_gives_its_title_and_authors_in_windows_1252() {
        // "Réglez-lui son compte ! – San-Antonio", é = 0xE9, the en dash = 0x96.
        let data = old_mobi(
            b"R\xe9glez-lui son compte ! \x96 1949",
            &[
                (EXTH_AUTHOR, b"San-Antonio"),
                (EXTH_AUTHOR, b"Fr\xe9d\xe9ric Dard"),
                (EXTH_AUTHOR, b"San-Antonio"),
            ],
        );
        assert_eq!(
            kindle_details_of(&data),
            Some(KindleDetails {
                title: "Réglez-lui son compte ! – 1949".into(),
                author: "San-Antonio & Frédéric Dard".into(),
                library_id: None,
            })
        );
        // EXTH 503 wins over the full name when there is one.
        let data = old_mobi(b"SHORT NAME", &[(EXTH_TITLE, b"The Real Title")]);
        assert_eq!(kindle_details_of(&data).unwrap().title, "The Real Title");
        // Nothing to say: empty fields, not a failure.
        assert_eq!(
            kindle_details_of(&old_mobi(b"", &[])),
            Some(KindleDetails {
                title: String::new(),
                author: String::new(),
                library_id: None
            })
        );
    }

    #[test]
    fn malformed_kindle_files_give_nothing() {
        assert_eq!(exth_string(b"", EXTH_ASIN), None);
        assert_eq!(exth_string(&[0u8; 100], EXTH_ASIN), None);
        // A plausible header whose record 0 points far past the end.
        let mut data = vec![0u8; 90];
        data[60..68].copy_from_slice(b"BOOKMOBI");
        data[78..82].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(exth_string(&data, EXTH_ASIN), None);
        // An EXTH record claiming a huge length.
        let mut data = vec![0u8; 400];
        data[60..68].copy_from_slice(b"BOOKMOBI");
        data[78..82].copy_from_slice(&100u32.to_be_bytes());
        data[116..120].copy_from_slice(b"MOBI");
        data[120..124].copy_from_slice(&0xE8u32.to_be_bytes());
        data[228..232].copy_from_slice(&0x40u32.to_be_bytes());
        let exth = 100 + 16 + 0xE8;
        data[exth..exth + 4].copy_from_slice(b"EXTH");
        data[exth + 8..exth + 12].copy_from_slice(&1u32.to_be_bytes());
        data[exth + 12..exth + 16].copy_from_slice(&EXTH_ASIN.to_be_bytes());
        data[exth + 16..exth + 20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(exth_string(&data, EXTH_ASIN), None);
    }

    #[test]
    fn record0_end_comes_from_the_record_table() {
        let mut head = vec![0u8; PDB_HEAD];
        head[76..78].copy_from_slice(&3u16.to_be_bytes());
        head[78..82].copy_from_slice(&200u32.to_be_bytes());
        head[86..90].copy_from_slice(&18_384u32.to_be_bytes());
        assert_eq!(record0_end(&head), Some(18_384));
        // One record only, or a record 1 inside the header: unknown.
        head[76..78].copy_from_slice(&1u16.to_be_bytes());
        assert_eq!(record0_end(&head), None);
        head[76..78].copy_from_slice(&3u16.to_be_bytes());
        head[86..90].copy_from_slice(&10u32.to_be_bytes());
        assert_eq!(record0_end(&head), None);
        assert_eq!(record0_end(&head[..40]), None);
    }

    #[test]
    fn epub_details_read_the_library_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Book.epub");
        minimal_epub(&path);
        assert_eq!(
            epub_details(&path),
            Some(("Test Book".into(), "Tester".into(), None))
        );
        crate::commands::library::epub_edit::write_library_id(&path, ID).unwrap();
        assert_eq!(epub_details(&path).unwrap().2.as_deref(), Some(ID));
        std::fs::write(&path, b"not a zip").unwrap();
        assert_eq!(epub_details(&path), None);
    }
}
