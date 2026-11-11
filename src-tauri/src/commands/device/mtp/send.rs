//! Sending one book to an MTP Kindle: the same job as for a drive
//! (`transfer.rs`), with the copy made over MTP.
//!
//! Steps 1–3 run on the calling thread, exactly as for a drive: the
//! **source** (from the library index when the job names the book), the
//! **payload** (converted if needed; an AZW3 carries the book id as its
//! ASIN, an EPUB copy an `urn:librarium:` identifier; temp files are
//! removed whatever happens), the **name**. Then one job on the device's
//! worker does the rest, so nothing else touches the device in between:
//!
//! 4. **Folder**: where the manifest says this book is (looked up freshly),
//!    else next to the title-matched `existing_device_path`, else
//!    `documents/<subfolder>`.
//! 5. **Name**: FAT-safe. A file of that name is replaced if it is this
//!    book or unknown to the manifest; if it is *another* library book, the
//!    copy is named `Name_2.ext` (never a duplicate with the same name, the
//!    `feat/mtp` branch's behaviour).
//! 6. **Space**: checked against the storage's free space, asked freshly.
//! 7. **Copy**: streamed from the file, with byte progress and cancellation.
//!    Replacing can't be atomic over MTP (nothing renames over a file).
//!    When the device can rename, the new copy is written under a hidden
//!    temp name (`.librarium-<random>.part`), then the old copy is deleted
//!    and the new one renamed: until the delete, the old copy is intact.
//!    When it can't, the old copy is deleted first (logged): if the copy
//!    then fails, the book is no longer on the device — the library still
//!    has it; send it again.
//! 8. **Manifest**, then the **superseded copy** (this book's previous copy
//!    under another name or in another folder) is deleted, under the same
//!    rules as on a drive.
//!
//! A cancel (or a pulled cable) mid-copy leaves a partial object; the
//! worker deletes it once reconnected (`mtp::worker`).

use std::cell::Cell;
use std::fs::File;
use std::path::{Path, PathBuf};

use super::super::listing::is_ebook;
use super::super::manifest::{path_key, ManifestEntry};
use super::super::transfer::{
    check_free_space, finish, prepare_payload, resolve_source, scale_progress, validate_subfolder,
    JobContext, JobOutcome, Stop, TransferStatus,
};
use super::{in_documents, manifest};
use crate::error::display_name;
use crate::fsutil;
use crate::models::device::TransferRequest;
use crate::mtp::devfs::{join, parent_of, same_name, DeviceFs, Folder, DOCUMENTS};
use crate::mtp::session::{MtpError, MtpSession, Object};
use crate::mtp::worker::{Connection, Ticker};
use crate::mtp::{self, MtpState};

/// Send one book to the MTP Kindle `job.device_path` (see the module docs),
/// reporting through `ctx` like [`super::super::transfer::transfer_one`].
pub(crate) fn transfer_one(
    job: &TransferRequest,
    ctx: &JobContext,
    state: &MtpState,
) -> JobOutcome {
    finish(job, ctx, run(job, ctx, state))
}

/// What the device job needs, owned (it runs on the worker thread).
struct Plan {
    /// The file to copy (the library file, or a temp payload the caller keeps alive).
    local: PathBuf,
    name: String,
    size: u64,
    /// Library id, title, author.
    identity: Option<(String, String, String)>,
    /// Under `documents/`, `/`-separated, already validated.
    subfolder: Option<String>,
    /// Device path of the title-matched copy the frontend sent.
    hint: Option<String>,
}

/// Where the book went, and what to log.
struct Sent {
    rel: String,
    notes: Vec<String>,
}

fn run(job: &TransferRequest, ctx: &JobContext, state: &MtpState) -> Result<PathBuf, Stop> {
    let check_cancel = || {
        if (ctx.is_cancelled)() {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let (key, _) = mtp::split_path(&job.device_path)
        .ok_or_else(|| format!("Not an MTP device: {}", job.device_path))?;
    let worker = state.worker(key).map_err(|e| match e {
        MtpError::Refused(message) => Stop::Failed(message),
        _ => Stop::Failed(
            "The Kindle (MTP) isn't connected any more. Plug it in again and retry.".into(),
        ),
    })?;

    let source = resolve_source(job, ctx.log)?;
    (ctx.log)(format!(
        "[send_books] job {} — src: {:?} book id: {} needs_conversion: {} target: {} (MTP)",
        job.id,
        source.path,
        source
            .identity
            .as_ref()
            .map_or("none (sent without identity)", |i| i.book_id.as_str()),
        job.needs_conversion,
        job.target_format
    ));
    if !source.path.is_file() {
        return Err(format!("Source file not found: {}", display_name(&source.path)).into());
    }
    let subfolder = validate_subfolder(job.subfolder.as_deref())?.map(|p| {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    });
    let (payload, name) = prepare_payload(&source, job, ctx)?;
    check_cancel()?;
    let size = std::fs::metadata(payload.path())
        .map_err(|e| format!("Cannot read {}: {e}", payload.path().display()))?
        .len();
    let hint = job
        .existing_device_path
        .as_deref()
        .and_then(mtp::split_path)
        .filter(|(k, rel)| *k == key && !rel.is_empty())
        .map(|(_, rel)| rel.to_string());
    let plan = Plan {
        local: payload.path().to_path_buf(),
        name,
        size,
        identity: source
            .identity
            .as_ref()
            .map(|i| (i.book_id.clone(), i.title.clone(), i.author.clone())),
        subfolder,
        hint,
    };

    let start = if job.needs_conversion { 60 } else { 30 };
    ctx.report(job, TransferStatus::Transferring, start, None);
    let last = Cell::new(start);
    let mut on_progress = |done: u64, total: u64| {
        let pct = scale_progress(start, done, total);
        if pct != last.get() {
            ctx.report(job, TransferStatus::Transferring, pct, None);
            last.set(pct);
        }
    };
    let result = worker.call_with_progress(
        move |c, ticker| send_book(c, &plan, ticker),
        &mut on_progress,
        ctx.is_cancelled,
    );
    // The worker is done reading it.
    drop(payload);
    match result {
        Ok(sent) => {
            for note in sent.notes {
                (ctx.log)(format!("[send_books] job {}: {note}", job.id));
            }
            Ok(PathBuf::from(mtp::file_path(key, &sent.rel)))
        }
        Err(MtpError::Cancelled) => Err(Stop::Cancelled),
        Err(e) => {
            (ctx.log)(format!("[send_books] job {}: MTP error: {e}", job.id));
            Err(Stop::Failed(e.user_message()))
        }
    }
}

/// Steps 4–8, on the worker thread.
fn send_book(c: &mut Connection, plan: &Plan, ticker: &Ticker) -> Result<Sent, MtpError> {
    if ticker.is_cancelled() {
        return Err(MtpError::Cancelled);
    }
    let s = &mut *c.session;
    let fs = &mut c.fs;
    let mut notes = Vec::new();
    let known = match manifest::load(fs, s) {
        Ok(stored) => stored.manifest().clone(),
        Err(e) if e.is_fatal() => return Err(e),
        Err(e) => {
            notes.push(format!("WARNING — the device manifest can't be read ({e}); the copy won't replace its previous one"));
            Default::default()
        }
    };
    let book_id = plan.identity.as_ref().map(|(id, _, _)| id.as_str());

    // 4. Folder.
    let previous = match book_id.and_then(|id| known.entry(id)) {
        Some(entry) => fs
            .resolve_checked(s, &entry.path)?
            .filter(|o| !o.is_folder)
            .map(|_| entry.path.clone()),
        None => None,
    };
    let mut dir_rel = None;
    for (what, rel) in [
        ("previous copy", previous.as_deref()),
        ("existing_device_path", plan.hint.as_deref()),
    ] {
        let Some(rel) = rel else { continue };
        let dir = parent_of(rel);
        if in_documents(dir) && fs.resolve_checked(s, dir)?.is_some_and(|o| o.is_folder) {
            dir_rel = Some(dir.to_string());
            break;
        }
        notes.push(format!(
            "ignoring the {what}'s folder (gone, or outside documents): {rel}"
        ));
    }
    let dir_rel = dir_rel.unwrap_or_else(|| {
        plan.subfolder
            .as_deref()
            .map_or(DOCUMENTS.to_string(), |sub| join(DOCUMENTS, sub))
    });
    let folder = fs.ensure_dir(s, &dir_rel)?;

    // 5. Name. (The cached listing, kept current with Librarium's own
    // changes; a file of that name in it is checked to still be there.)
    let name = fsutil::sanitize_filename(&plan.name);
    let (siblings, existing) = fs.find_child(s, &folder, &name)?;
    let (final_name, replace) = match existing {
        Some(o) => {
            let another_book = book_id.is_some_and(|id| {
                known
                    .owner_of(&join(&dir_rel, &o.name))
                    .is_some_and(|owner| owner != id)
            });
            if o.is_folder || another_book {
                (
                    fsutil::unique_name(&name, |n| siblings.iter().any(|x| same_name(&x.name, n))),
                    None,
                )
            } else {
                (name, Some(o))
            }
        }
        None => (name, None),
    };

    // 6. Space.
    let (free, _) = fs.space(s, &folder.storage)?;
    check_free_space(plan.size, free).map_err(MtpError::Refused)?;

    // 7. Copy.
    let sent = copy(
        fs,
        s,
        &folder,
        &final_name,
        plan,
        replace.as_ref(),
        ticker,
        &mut notes,
    )?;
    let new_rel = join(&dir_rel, &sent.name);
    c.space = fs.space(s, &folder.storage).ok();

    // 8. Manifest, superseded copy.
    if let Some((id, title, author)) = &plan.identity {
        let entry = ManifestEntry {
            path: new_rel.clone(),
            title: title.clone(),
            author: author.clone(),
            format: Path::new(&sent.name)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            size_bytes: plan.size,
            sent_at: chrono::Utc::now().to_rfc3339(),
        };
        let recorded = manifest::update(fs, s, |m| {
            let previous = m.books.remove(id);
            let replaced = m.remove_path(&new_rel);
            m.books.insert(id.clone(), entry);
            let shared = previous
                .as_ref()
                .is_some_and(|p| m.owner_of(&p.path).is_some_and(|o| o != id));
            (previous, replaced, shared)
        });
        match recorded {
            Ok((previous, replaced, shared)) => {
                if !replaced.is_empty() {
                    notes.push(format!("{} replaced the file recorded for {}", sent.name, replaced.join(", ")));
                }
                if let Some(previous) = previous.filter(|p| !shared && path_key(&p.path) != path_key(&new_rel)) {
                    delete_superseded(fs, s, &previous.path, &sent, &mut notes);
                }
            }
            Err(e) => notes.push(format!(
                "WARNING — {} is on the Kindle, but {}; its previous copy (if any) was left in place",
                sent.name,
                e.user_message()
            )),
        }
    }
    Ok(Sent {
        rel: new_rel,
        notes,
    })
}

/// Step 7 (see the module docs): the new object.
#[allow(clippy::too_many_arguments)]
fn copy(
    fs: &mut DeviceFs,
    s: &mut dyn MtpSession,
    folder: &Folder,
    name: &str,
    plan: &Plan,
    replace: Option<&Object>,
    ticker: &Ticker,
    notes: &mut Vec<String>,
) -> Result<Object, MtpError> {
    let send =
        |s: &mut dyn MtpSession, fs: &mut DeviceFs, as_name: &str| -> Result<Object, MtpError> {
            let mut file = File::open(&plan.local)
                .map_err(|e| MtpError::Local(format!("Could not read the book to send: {e}")))?;
            let mut progress = |done: u64| {
                ticker.progress(done, plan.size);
                !ticker.is_cancelled()
            };
            let object = s.send_file(
                &folder.storage,
                folder.id.as_deref(),
                as_name,
                plan.size,
                &mut file,
                &mut progress,
            )?;
            fs.note_added(&object);
            Ok(object)
        };
    let Some(old) = replace else {
        return send(s, fs, name);
    };

    // TODO(device-validation): check that a Kindle accepts hidden temp
    // names and renames them (MTP SetObjectPropValue ObjectFileName).
    if s.can_rename() {
        let temp = format!(
            ".librarium-{}.part",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        match send(s, fs, &temp) {
            Ok(new) => {
                if let Err(e) = s.delete(&old.id) {
                    // Keep the old copy; drop the new one.
                    if s.delete(&new.id).is_ok() {
                        fs.note_removed(&new);
                    }
                    return Err(e);
                }
                fs.note_removed(old);
                return match s.rename(&new.id, name) {
                    Ok(()) => Ok(fs.note_renamed(&new, name)),
                    Err(e) if e.is_fatal() => Err(e),
                    Err(e) => {
                        notes.push(format!("the Kindle refused to rename the new copy ({e}); sending it again under its name"));
                        let object = send(s, fs, name)?;
                        if s.delete(&new.id).is_ok() {
                            fs.note_removed(&new);
                        }
                        Ok(object)
                    }
                };
            }
            Err(e) if e.is_fatal() || e == MtpError::Cancelled => return Err(e),
            Err(e) => notes.push(format!(
                "the Kindle refused a temporary file ({e}); replacing the old copy directly"
            )),
        }
    }
    notes.push(format!("{name}: the old copy was deleted before sending the new one (this Kindle can't rename files over MTP)"));
    s.delete(&old.id)?;
    fs.note_removed(old);
    send(s, fs, name)
}

/// Delete this book's previous copy at `rel` (another name or folder),
/// only if it is a book file inside `documents/` and not the file just
/// written. Failures are logged: a leftover duplicate is harmless.
fn delete_superseded(
    fs: &mut DeviceFs,
    s: &mut dyn MtpSession,
    rel: &str,
    sent: &Object,
    notes: &mut Vec<String>,
) {
    if !in_documents(rel) || !is_ebook(Path::new(rel)) {
        return;
    }
    match fs.resolve_checked(s, rel) {
        Ok(Some(old)) if !old.is_folder && old.id != sent.id => match s.delete(&old.id) {
            Ok(()) => {
                fs.note_removed(&old);
                notes.push(format!("removed the previous copy {rel}"));
            }
            Err(e) => notes.push(format!(
                "WARNING — could not remove the previous copy {rel}: {e}"
            )),
        },
        Ok(_) => {}
        Err(e) => notes.push(format!(
            "WARNING — could not look for the previous copy {rel}: {e}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::device::identity::{epub_details, kindle_details};
    use crate::commands::device::mtp::tests::{connected, record, KEY};
    use crate::commands::device::tests::minimal_epub;
    use crate::commands::device::transfer::TransferProgress;
    use crate::commands::library::test_support::{scan, write_epub};
    use crate::logging::LogLines;
    use crate::mtp::fake::FakeDevice;
    use std::cell::RefCell;
    use std::time::Duration;

    fn job(src: &Path) -> TransferRequest {
        TransferRequest {
            id: "job-1".into(),
            book_path: src.to_string_lossy().into_owned(),
            needs_conversion: false,
            target_format: "azw3".into(),
            device_path: mtp::mount_path(KEY),
            subfolder: None,
            existing_device_path: None,
            book_id: None,
            library_path: None,
        }
    }

    fn library_job(lib: &Path, file_name: &str) -> (TransferRequest, String) {
        let book = scan(lib).remove(file_name).unwrap();
        let mut j = job(Path::new(&book.path));
        j.book_id = Some(book.id.clone());
        j.library_path = Some(lib.to_string_lossy().into_owned());
        (j, book.id)
    }

    struct Harness {
        events: RefCell<Vec<TransferProgress>>,
        /// Cancel once the copy has made progress.
        cancel_mid_copy: bool,
        temp: tempfile::TempDir,
        lines: LogLines,
    }

    impl Harness {
        fn new() -> Self {
            Harness {
                events: RefCell::new(vec![]),
                cancel_mid_copy: false,
                temp: tempfile::tempdir().unwrap(),
                lines: LogLines::default(),
            }
        }

        fn run(&self, job: &TransferRequest, state: &MtpState) -> JobOutcome {
            let is_cancelled = || {
                self.cancel_mid_copy
                    && self
                        .events
                        .borrow()
                        .iter()
                        .any(|e| e.status == TransferStatus::Transferring && e.progress > 31)
            };
            let emit = |p: TransferProgress| self.events.borrow_mut().push(p);
            let free = |_: &Path| None;
            let log = self.lines.sink();
            let ctx = JobContext {
                is_cancelled: &is_cancelled,
                emit: &emit,
                log: &log,
                free_space: &free,
                temp_dir: self.temp.path(),
            };
            transfer_one(job, &ctx, state)
        }

        fn statuses(&self) -> Vec<TransferStatus> {
            self.events.borrow().iter().map(|e| e.status).collect()
        }

        fn temp_is_empty(&self) -> bool {
            std::fs::read_dir(self.temp.path())
                .unwrap()
                .next()
                .is_none()
        }
    }

    fn done(outcome: JobOutcome) -> String {
        match outcome {
            JobOutcome::Done(p) => p.to_string_lossy().into_owned(),
            other => panic!("expected done, got {other:?}"),
        }
    }

    fn manifest_of(state: &MtpState) -> crate::commands::device::manifest::DeviceManifest {
        state
            .worker(KEY)
            .unwrap()
            .call(|c| manifest::load(&mut c.fs, &mut *c.session))
            .unwrap()
            .manifest()
            .clone()
    }

    fn documents(device: &FakeDevice) -> Vec<String> {
        device
            .paths()
            .into_iter()
            .filter(|p| p.starts_with("documents/"))
            .collect()
    }

    #[test]
    fn a_file_is_streamed_with_byte_progress_and_a_fat_safe_name() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune: Messiah?.azw3");
        let content: Vec<u8> = (0..700_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(&src, &content).unwrap();

        let h = Harness::new();
        assert_eq!(
            done(h.run(&job(&src), &state)),
            "mtp://usb-1-2/documents/Dune_ Messiah_.azw3"
        );
        assert_eq!(
            device.read("documents/Dune_ Messiah_.azw3").unwrap(),
            content
        );
        let events = h.events.borrow();
        assert!(
            events.windows(2).all(|w| w[0].progress <= w[1].progress),
            "never backwards"
        );
        assert!(
            events
                .iter()
                .filter(|e| e.status == TransferStatus::Transferring)
                .count()
                > 3,
            "byte progress"
        );
        assert_eq!(
            events.last().map(|e| (e.status, e.progress)),
            Some((TransferStatus::Done, 100))
        );
        assert!(
            manifest_of(&state).books.is_empty(),
            "nothing recorded without an identity"
        );
        assert!(
            state.ready(KEY).unwrap().free_bytes.unwrap() < 8 << 30,
            "the free space is updated"
        );
    }

    #[test]
    fn a_library_book_carries_its_id_and_is_recorded() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let (mut j, id) = library_job(lib.path(), "Book.epub");
        j.needs_conversion = true;

        let h = Harness::new();
        assert_eq!(done(h.run(&j, &state)), "mtp://usb-1-2/documents/Book.azw3");
        assert!(h.temp_is_empty(), "the conversion temp file is removed");
        assert_eq!(h.statuses().first(), Some(&TransferStatus::Converting));
        let copy = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(copy.path(), device.read("documents/Book.azw3").unwrap()).unwrap();
        assert_eq!(
            kindle_details(copy.path())
                .and_then(|d| d.library_id)
                .as_deref(),
            Some(id.as_str()),
            "the ASIN is the book id"
        );
        let entry = manifest_of(&state).entry(&id).cloned().unwrap();
        assert_eq!(
            (entry.path.as_str(), entry.format.as_str()),
            ("documents/Book.azw3", "azw3")
        );
    }

    #[test]
    fn an_epub_copy_is_tagged_and_the_library_file_untouched() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let original = std::fs::read(lib.path().join("Dune.epub")).unwrap();
        let (j, id) = library_job(lib.path(), "Dune.epub");

        let h = Harness::new();
        done(h.run(&j, &state));
        let copy = tempfile::Builder::new().suffix(".epub").tempfile().unwrap();
        std::fs::write(copy.path(), device.read("documents/Dune.epub").unwrap()).unwrap();
        assert_eq!(
            epub_details(copy.path()).unwrap().2.as_deref(),
            Some(id.as_str())
        );
        assert_eq!(
            std::fs::read(lib.path().join("Dune.epub")).unwrap(),
            original
        );
        assert!(h.temp_is_empty());
    }

    #[test]
    fn resending_replaces_through_a_temp_copy_and_a_rename() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let (j, id) = library_job(lib.path(), "Dune.epub");
        done(Harness::new().run(&j, &state));
        device.state().log.clear();

        done(Harness::new().run(&j, &state));
        assert_eq!(
            documents(&device),
            vec!["documents/Dune.epub"],
            "one copy, no duplicate"
        );
        let log = device.state().log.clone();
        assert!(
            log[0].starts_with("send .librarium-")
                && log[1] == "delete Dune.epub"
                && log[2].ends_with("-> Dune.epub"),
            "{log:?}"
        );
        assert_eq!(
            manifest_of(&state).entry(&id).unwrap().path,
            "documents/Dune.epub"
        );
    }

    #[test]
    fn without_rename_the_old_copy_is_deleted_first() {
        let device = FakeDevice::kindle();
        device.state().can_rename = false;
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        std::fs::write(&src, b"new").unwrap();
        device.put("documents/Dune.azw3", b"old");

        let h = Harness::new();
        done(h.run(&job(&src), &state));
        assert_eq!(device.read("documents/Dune.azw3").unwrap(), b"new");
        assert_eq!(documents(&device), vec!["documents/Dune.azw3"]);
        assert!(h.lines.contains("old copy was deleted before sending"));
    }

    #[test]
    fn another_library_books_file_is_never_overwritten() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        device.put("documents/Dune.epub", b"someone else's Dune");
        record(
            &state,
            "bbbbbbbb-0000-4000-8000-000000000002",
            "documents/Dune.epub",
        );
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let (j, id) = library_job(lib.path(), "Dune.epub");

        assert_eq!(
            done(Harness::new().run(&j, &state)),
            "mtp://usb-1-2/documents/Dune_2.epub"
        );
        assert_eq!(
            device.read("documents/Dune.epub").unwrap(),
            b"someone else's Dune"
        );
        let m = manifest_of(&state);
        assert_eq!(m.entry(&id).unwrap().path, "documents/Dune_2.epub");
        assert_eq!(m.books.len(), 2);
    }

    #[test]
    fn a_renamed_book_replaces_its_previous_copy_in_its_folder() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let (mut j, id) = library_job(lib.path(), "Dune.epub");
        device.put("documents/Shelf/Old name.epub", b"old copy");
        device.put("documents/Shelf/Old name.sdr/x.mbs", b"position");
        device.put("documents/Shelf/Unrelated.epub", b"keep");
        record(&state, &id, "documents/Shelf/Old name.epub");
        j.subfolder = Some("Elsewhere".into());

        let h = Harness::new();
        assert_eq!(
            done(h.run(&j, &state)),
            "mtp://usb-1-2/documents/Shelf/Dune.epub"
        );
        assert_eq!(
            documents(&device),
            vec![
                "documents/Shelf",
                "documents/Shelf/Dune.epub",
                "documents/Shelf/Old name.sdr",
                "documents/Shelf/Old name.sdr/x.mbs",
                "documents/Shelf/Unrelated.epub",
            ]
        );
        assert!(h.lines.contains("removed the previous copy"));
        assert_eq!(
            manifest_of(&state).entry(&id).unwrap().path,
            "documents/Shelf/Dune.epub"
        );
    }

    #[test]
    fn subfolders_and_title_matched_folders() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        std::fs::write(&src, b"dune").unwrap();
        let mut j = job(&src);
        j.subfolder = Some("Sci-Fi/Herbert: Dune".into());
        assert_eq!(
            done(Harness::new().run(&j, &state)),
            "mtp://usb-1-2/documents/Sci-Fi/Herbert_ Dune/Dune.azw3"
        );

        device.put("documents/Shelf/Dune - Someone Else.epub", b"other");
        j.subfolder = None;
        j.existing_device_path =
            Some("mtp://usb-1-2/documents/Shelf/Dune - Someone Else.epub".into());
        assert_eq!(
            done(Harness::new().run(&j, &state)),
            "mtp://usb-1-2/documents/Shelf/Dune.azw3"
        );
        assert_eq!(
            device
                .read("documents/Shelf/Dune - Someone Else.epub")
                .unwrap(),
            b"other",
            "never deleted"
        );

        j.subfolder = Some("../../escape".into());
        assert!(matches!(
            Harness::new().run(&j, &state),
            JobOutcome::Failed(_)
        ));
    }

    #[test]
    fn a_cancelled_replace_keeps_the_old_copy_and_leaves_nothing_behind() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Big.azw3");
        std::fs::write(&src, vec![9u8; 2 << 20]).unwrap();
        device.put("documents/Big.azw3", b"previous");
        device.state().chunk_delay = Duration::from_millis(15);

        let mut h = Harness::new();
        h.cancel_mid_copy = true;
        assert_eq!(h.run(&job(&src), &state), JobOutcome::Cancelled);
        assert_eq!(h.statuses().last(), Some(&TransferStatus::Cancelled));
        device.state().chunk_delay = Duration::ZERO;
        // The worker reconnected and removed the partial temp copy.
        assert!(state
            .worker(KEY)
            .unwrap()
            .call(|c| c.session.storages())
            .is_ok());
        assert_eq!(documents(&device), vec!["documents/Big.azw3"]);
        assert_eq!(device.read("documents/Big.azw3").unwrap(), b"previous");
    }

    #[test]
    fn a_full_device_fails_before_copying() {
        let device = FakeDevice::kindle();
        device.state().capacity = 1000;
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Big.pdf");
        std::fs::write(&src, vec![0u8; 4096]).unwrap();
        let JobOutcome::Failed(msg) = Harness::new().run(&job(&src), &state) else {
            panic!("expected a failure")
        };
        assert!(msg.starts_with("Not enough space on the device"), "{msg}");
        assert!(documents(&device).is_empty());
    }

    #[test]
    fn a_pulled_cable_fails_the_job_and_the_partial_file_goes_on_reconnection() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Big.azw3");
        std::fs::write(&src, vec![1u8; 500_000]).unwrap();
        device.state().fail_send_after = Some(200_000);

        let JobOutcome::Failed(msg) = Harness::new().run(&job(&src), &state) else {
            panic!("expected a failure")
        };
        assert!(msg.contains("disconnected"), "{msg}");
        assert!(
            state
                .worker(KEY)
                .unwrap()
                .call(|c| c.session.storages())
                .is_ok(),
            "reconnected"
        );
        assert!(
            documents(&device).is_empty(),
            "the partial file was removed"
        );
        // And the next try works.
        done(Harness::new().run(&job(&src), &state));
        assert_eq!(device.read("documents/Big.azw3").unwrap().len(), 500_000);
    }

    #[test]
    fn refused_while_disabled_or_unplugged() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        std::fs::write(&src, b"dune").unwrap();
        state.set_enabled(false);
        let JobOutcome::Failed(msg) = Harness::new().run(&job(&src), &state) else {
            panic!()
        };
        assert!(msg.contains("Settings"), "{msg}");
        state.set_enabled(true);
        let mut other = job(&src);
        other.device_path = "mtp://usb-9-9".into();
        let JobOutcome::Failed(msg) = Harness::new().run(&other, &state) else {
            panic!()
        };
        assert!(msg.contains("isn't connected"), "{msg}");
    }
}
