//! The venue editor of the watch grid (design 2026-11-11, item 2).
//!
//! `v` opens one input line, prefilled with the venues of the selected
//! target. Enter checks the line with [`parse_venues`], the one check the
//! wizard also uses, and queues the change for the run. The run takes the
//! queue at the start of its next tick ([`VenueQueue`]), so no tick reads
//! two venue sets. `u` undoes the last edit (one level) and `Ctrl-S` saves
//! the list to its file, unless the file changed on disk since the run
//! started.
//!
//! The editor changes the run, never the saved watchlist, until `Ctrl-S`.
//! Every outcome leaves a [`VenueRecord`] for `tui-events.jsonl`; the app
//! takes them with [`VenueEditor::take_records`].

use super::journal::VenueRecord;
use super::wizard::{LineKey, edit_line, parse_venues};
use crate::cli::textfit::{cols, cut_with_ellipsis};
use crate::cli::theme::{ACCENT, ERR, OK, SECONDARY, WARN};
use crate::cli::watch::run::{VenueChange, VenueQueue};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use scope::watch::{Target, TargetKind, Watchlist};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The key that opens the editor.
pub const EDIT_KEY: char = 'v';
/// The key that undoes the last edit.
pub const UNDO_KEY: char = 'u';

/// Why `v` does nothing after the run ended.
pub const RUN_ENDED_NOTE: &str =
    "the run ended: change the venues in the wizard (monitor) and start again";
/// Why `v` does nothing before there is a row to select.
pub const NO_ROW_NOTE: &str = "no row is selected yet: wait for the first tick";
/// Why `u` does nothing.
pub const NOTHING_TO_UNDO_NOTE: &str = "nothing to undo";
/// Why a save waits.
pub const SAVE_PENDING_NOTE: &str =
    "a venue change has not reached the run: save after the next tick";
/// The footer text for a prompt that must say what it edits.
const PROMPT: &str = "venues";
/// The end of the prompt line: the cursor.
const CURSOR: &str = "█";
/// The end of the legend while the line is open.
const LINE_KEYS: &str = " · Enter apply · Esc cancel";
/// Why an edit cannot apply once the run ended.
const REASON_RUN_ENDED: &str = "run ended before the change applied";

/// The outcomes of a [`VenueRecord`].
pub mod outcome {
    pub const QUEUED: &str = "queued";
    pub const APPLIED: &str = "applied";
    pub const CANCELLED: &str = "cancelled";
    pub const REFUSED: &str = "refused";
    pub const SUPERSEDED: &str = "superseded";
    pub const UNDONE: &str = "undone";
    pub const SAVED: &str = "saved";
    pub const SAVE_REFUSED: &str = "save_refused";
}

/// The file of a watchlist, and the text it had when the run started: the
/// guard of `Ctrl-S`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListFile {
    pub path: PathBuf,
    /// The text read at the start; `None`: there was no file.
    loaded: Option<String>,
}

impl ListFile {
    /// The file at `path`, as it is now. An unreadable file is as a missing
    /// one: the guard then sees any later text as a change.
    pub fn read(path: PathBuf) -> Self {
        let loaded = std::fs::read_to_string(&path).ok();
        Self { path, loaded }
    }
}

/// What the editor reads from the grid at each call.
pub struct Ctx<'a> {
    /// The list as the user gave it, with the venues the run has applied.
    pub list: &'a Watchlist,
    /// The run number of the last tick.
    pub run: u64,
    pub running: bool,
}

/// How a message shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Warn,
    Err,
}

impl Tone {
    fn style(self) -> Style {
        match self {
            Tone::Info => OK,
            Tone::Warn => WARN,
            Tone::Err => ERR,
        }
    }
}

/// The line being typed.
#[derive(Debug, Clone)]
struct Line {
    target: usize,
    text: String,
    /// Why the last Enter was refused; the line stays open.
    error: Option<String>,
}

/// An edit that waits for the run.
#[derive(Debug, Clone)]
struct Pending {
    seq: u64,
    target: usize,
    from: Vec<String>,
    to: Vec<String>,
    /// The line as typed; `None` for an undo.
    line: Option<String>,
    queued_at: String,
}

/// What `u` restores.
#[derive(Debug, Clone)]
struct Undo {
    target: usize,
    before: Vec<String>,
}

/// The editor's state.
#[derive(Debug)]
pub struct VenueEditor {
    known: Vec<String>,
    queue: VenueQueue,
    line: Option<Line>,
    pending: Vec<Pending>,
    undo: Option<Undo>,
    next_seq: u64,
    file: Option<ListFile>,
    /// The venues of each target as the file has them: at the start, and
    /// after each save.
    baseline: Vec<Vec<String>>,
    notice: Option<(String, Tone)>,
    records: Vec<VenueRecord>,
}

/// `venues` as one line of words.
fn words(venues: &[String]) -> String {
    venues.join(" ")
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn sha256(text: Option<&str>) -> Option<String> {
    text.map(|t| hex::encode(Sha256::digest(t.as_bytes())))
}

impl VenueEditor {
    /// An editor over `list`: its venues are what the file has.
    pub fn new(list: &Watchlist) -> Self {
        Self {
            known: Vec::new(),
            queue: VenueQueue::default(),
            line: None,
            pending: Vec::new(),
            undo: None,
            next_seq: 0,
            file: None,
            baseline: list.targets.iter().map(|t| t.venues.clone()).collect(),
            notice: None,
            records: Vec::new(),
        }
    }

    /// The venue IDs a line may name, and the file of the list.
    pub fn with(mut self, known: Vec<String>, file: Option<ListFile>) -> Self {
        self.known = known;
        self.file = file;
        self
    }

    /// The queue the run reads at the start of each tick.
    pub fn queue(&self) -> VenueQueue {
        self.queue.clone()
    }

    /// Whether the input line is open: then every key types into it.
    pub fn is_open(&self) -> bool {
        self.line.is_some()
    }

    /// Whether an edit waits for the run.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Whether `u` has something to undo.
    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    /// Whether the list has venues that the file does not.
    pub fn has_unsaved(&self, list: &Watchlist) -> bool {
        !self.unsaved(list).is_empty()
    }

    /// The records since the last call, oldest first.
    pub fn take_records(&mut self) -> Vec<VenueRecord> {
        std::mem::take(&mut self.records)
    }

    /// Drops the message of the last key.
    pub fn clear_notice(&mut self) {
        self.notice = None;
    }

    /// The ids of the targets whose venues the file does not have.
    fn unsaved<'a>(&self, list: &'a Watchlist) -> Vec<&'a str> {
        list.targets
            .iter()
            .zip(&self.baseline)
            .filter(|(t, saved)| &t.venues != *saved)
            .map(|(t, _)| t.id.as_str())
            .collect()
    }

    /// The closing line of the grid when venues changed and were not saved.
    pub fn summary(&self, list: &Watchlist) -> Option<String> {
        let ids = self.unsaved(list);
        (!ids.is_empty()).then(|| {
            format!(
                "venues of {} changed in the run and not saved",
                ids.join(", ")
            )
        })
    }

    /// The venues a target has once the edits that wait have applied.
    fn effective(&self, list: &Watchlist, target: usize) -> Vec<String> {
        match self.pending.iter().find(|p| p.target == target) {
            Some(p) => p.to.clone(),
            None => list
                .targets
                .get(target)
                .map(|t| t.venues.clone())
                .unwrap_or_default(),
        }
    }

    fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.notice = Some((text.into(), tone));
    }

    fn push(
        &mut self,
        ctx: &Ctx,
        outcome: &'static str,
        target: Option<usize>,
        fill: impl FnOnce(&mut VenueRecord),
    ) {
        let mut r = VenueRecord::new(outcome, ctx.list.name.clone(), ctx.run, now());
        r.target = target
            .and_then(|i| ctx.list.targets.get(i))
            .map(|t| t.id.clone());
        fill(&mut r);
        self.records.push(r);
    }

    /// A record of an edit: its number, venues, line and queue time.
    fn push_edit(
        &mut self,
        ctx: &Ctx,
        outcome: &'static str,
        p: &Pending,
        fill: impl FnOnce(&mut VenueRecord),
    ) {
        self.push(ctx, outcome, Some(p.target), |r| {
            r.seq = Some(p.seq);
            r.from.clone_from(&p.from);
            r.to.clone_from(&p.to);
            r.line.clone_from(&p.line);
            r.queued_at = Some(p.queued_at.clone());
            fill(r);
        });
    }

    /// `v`: opens the line for `target`, the index of the selected row's
    /// target in the list, or says why not.
    pub fn open(&mut self, ctx: &Ctx, target: Option<usize>) {
        let refusal = match target.and_then(|i| ctx.list.targets.get(i).map(|t| (i, t))) {
            _ if !ctx.running => Err((RUN_ENDED_NOTE.to_string(), None)),
            None => Err((NO_ROW_NOTE.to_string(), None)),
            Some((i, t)) => match unsuited(t) {
                Some(why) => Err((why, Some(i))),
                None => Ok(i),
            },
        };
        match refusal {
            Ok(i) => {
                self.line = Some(Line {
                    target: i,
                    text: words(&self.effective(ctx.list, i)),
                    error: None,
                });
            }
            Err((why, target)) => {
                self.push(ctx, outcome::REFUSED, target, |r| {
                    r.reason = Some(why.clone());
                });
                self.say(why, Tone::Warn);
            }
        }
    }

    /// A key while the line is open.
    pub fn line_key(&mut self, ctx: &Ctx, key: KeyEvent) {
        let Some(mut line) = self.line.take() else {
            return;
        };
        // Ctrl-C closes the line, as Esc does, and does not stop the run.
        let cancel =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        let result = if cancel {
            LineKey::Cancel
        } else {
            edit_line(&mut line.text, key)
        };
        match result {
            LineKey::Open => {
                line.error = None;
                self.line = Some(line);
            }
            LineKey::Cancel => self.cancel(ctx, &line, "cancelled by the user"),
            LineKey::Submit => self.submit(ctx, line),
        }
    }

    fn cancel(&mut self, ctx: &Ctx, line: &Line, why: &str) {
        let from = self.effective(ctx.list, line.target);
        self.push(ctx, outcome::CANCELLED, Some(line.target), |r| {
            r.from = from;
            r.line = Some(line.text.clone());
            r.reason = Some(why.to_string());
        });
    }

    fn submit(&mut self, ctx: &Ctx, mut line: Line) {
        let from = self.effective(ctx.list, line.target);
        let refuse = |this: &mut Self, mut line: Line, why: String| {
            this.push(ctx, outcome::REFUSED, Some(line.target), |r| {
                r.from.clone_from(&from);
                r.line = Some(line.text.clone());
                r.reason = Some(why.clone());
            });
            line.error = Some(why);
            this.line = Some(line);
        };
        if !ctx.running {
            return refuse(self, line, RUN_ENDED_NOTE.to_string());
        }
        let Some(target) = ctx.list.targets.get(line.target) else {
            return refuse(self, line, NO_ROW_NOTE.to_string());
        };
        let needs = match target.kind {
            TargetKind::Pair => "a pair",
            _ => "a token with a market rule",
        };
        let typed: Vec<&str> = line.text.split_whitespace().collect();
        match parse_venues(&typed, &self.known, &from, Some(needs)) {
            Err(why) => refuse(self, line, why),
            Ok(to) if to == from => {
                self.cancel(ctx, &line, "no change");
                self.say("the venues are the same: nothing to change", Tone::Warn);
            }
            Ok(to) => {
                let text = std::mem::take(&mut line.text);
                let before = self.queue_change(ctx, line.target, to, Some(text));
                self.undo = Some(Undo {
                    target: line.target,
                    before,
                });
            }
        }
    }

    /// Queues the venues `to` for `target`. A change of the same target that
    /// waits is replaced, and recorded as such. Returns the venues before.
    fn queue_change(
        &mut self,
        ctx: &Ctx,
        target: usize,
        to: Vec<String>,
        line: Option<String>,
    ) -> Vec<String> {
        let from = self.effective(ctx.list, target);
        self.next_seq += 1;
        let change = Pending {
            seq: self.next_seq,
            target,
            from: from.clone(),
            to: to.clone(),
            line,
            queued_at: now(),
        };
        if let Some(i) = self.pending.iter().position(|p| p.target == target) {
            let old = self.pending.remove(i);
            self.push_edit(ctx, outcome::SUPERSEDED, &old, |r| {
                r.reason = Some(format!("replaced by edit {}", change.seq));
            });
        }
        self.queue.submit(VenueChange {
            seq: change.seq,
            target,
            venues: to,
        });
        self.push_edit(ctx, outcome::QUEUED, &change, |_| {});
        self.pending.push(change);
        from
    }

    /// Records and forgets the edits that wait: the run took no more
    /// changes.
    fn refuse_pending(&mut self, ctx: &Ctx) {
        for p in std::mem::take(&mut self.pending) {
            self.queue.withdraw(p.seq);
            self.push_edit(ctx, outcome::REFUSED, &p, |r| {
                r.reason = Some(REASON_RUN_ENDED.to_string());
            });
        }
    }

    /// The run ended: the line closes and the edits that did not apply are
    /// refused, in the record.
    pub fn ended(&mut self, ctx: &Ctx) {
        if let Some(line) = self.line.take() {
            self.cancel(ctx, &line, REASON_RUN_ENDED);
        }
        self.refuse_pending(ctx);
    }

    /// `u`: undoes the last edit, of any target, as a new edit of the old
    /// venues. An edit that has not reached the run is just taken back. A
    /// venue that comes back is fresh: its memory was dropped.
    pub fn undo(&mut self, ctx: &Ctx) {
        if !ctx.running {
            self.say(RUN_ENDED_NOTE, Tone::Warn);
            return;
        }
        let Some(undo) = self.undo.take() else {
            self.say(NOTHING_TO_UNDO_NOTE, Tone::Warn);
            return;
        };
        let current = self.effective(ctx.list, undo.target);
        let run_has = ctx
            .list
            .targets
            .get(undo.target)
            .map(|t| t.venues.clone())
            .unwrap_or_default();
        let waiting = self.pending.iter().position(|p| p.target == undo.target);
        let untouched = waiting
            .is_some_and(|i| undo.before == run_has && self.queue.withdraw(self.pending[i].seq));
        if untouched {
            let p = waiting.map(|i| self.pending.remove(i));
            if let Some(p) = p {
                self.push_edit(ctx, outcome::UNDONE, &p, |r| {
                    r.reason = Some("the change had not reached the run".to_string());
                    std::mem::swap(&mut r.from, &mut r.to);
                });
            }
            self.say("undone: the change had not reached the run", Tone::Info);
            return;
        }
        self.queue_change(ctx, undo.target, undo.before.clone(), None);
        let seq = self.next_seq;
        let before = undo.before;
        self.push(ctx, outcome::UNDONE, Some(undo.target), |r| {
            r.seq = Some(seq);
            r.from.clone_from(&current);
            r.to.clone_from(&before);
        });
        self.say(undo_note(&current, &before), Tone::Info);
    }

    /// The run applied `changes` at the start of the tick `run`. The grid
    /// has put the venues in its list.
    pub fn applied(&mut self, ctx: &Ctx, run: u64, changes: &[VenueChange]) {
        for c in changes {
            let Some(i) = self.pending.iter().position(|p| p.seq == c.seq) else {
                continue;
            };
            let p = self.pending.remove(i);
            self.push_edit(ctx, outcome::APPLIED, &p, |r| r.applied_at_run = Some(run));
        }
    }

    /// `Ctrl-S`: writes the list to its file, unless an edit waits or the
    /// file changed on disk since the run started.
    pub fn save(&mut self, ctx: &Ctx) {
        let path = self.file.as_ref().map_or_else(
            || scope::watch::file::path_in(&scope::paths::watchlists_dir(), &ctx.list.name),
            |f| f.path.clone(),
        );
        let on_disk = std::fs::read_to_string(&path).ok();
        let refuse =
            |this: &mut Self, why: String, hashes: Option<(Option<String>, Option<String>)>| {
                this.push(ctx, outcome::SAVE_REFUSED, None, |r| {
                    r.path = Some(path.display().to_string());
                    r.reason = Some(why.clone());
                    if let Some((loaded, disk)) = hashes {
                        r.loaded_sha256 = loaded;
                        r.disk_sha256 = disk;
                    }
                });
                this.say(why, Tone::Err);
            };
        if self.has_pending() {
            return refuse(self, SAVE_PENDING_NOTE.to_string(), None);
        }
        if let Some(file) = &self.file
            && file.loaded != on_disk
        {
            let why = format!(
                "{} changed on disk since the run started: not saved",
                file_name(&path)
            );
            let hashes = (sha256(file.loaded.as_deref()), sha256(on_disk.as_deref()));
            return refuse(self, why, Some(hashes));
        }
        if let Err(e) = scope::watch::file::save(&path, ctx.list) {
            return refuse(
                self,
                format!("could not save {}: {e}", path.display()),
                None,
            );
        }
        let comments = on_disk
            .as_deref()
            .is_some_and(|t| t.lines().any(|l| l.trim_start().starts_with('#')));
        let text = scope::watch::file::to_text(ctx.list);
        self.file = Some(ListFile {
            path: path.clone(),
            loaded: Some(text),
        });
        self.baseline = ctx.list.targets.iter().map(|t| t.venues.clone()).collect();
        self.push(ctx, outcome::SAVED, None, |r| {
            r.path = Some(path.display().to_string());
        });
        let kept = if comments {
            " (comments in the file are not kept)"
        } else {
            ""
        };
        self.say(format!("saved {}{kept}", file_name(&path)), Tone::Info);
    }

    // ---------------------------------------------------------------- draw

    /// The line above the legend, as wide as `width` at most: the input
    /// line, the message of the last key, an edit that waits, or the venues
    /// that are not saved.
    pub fn note(&self, list: &Watchlist, width: usize) -> Option<(String, Style)> {
        if let Some(line) = &self.line {
            let id = list.targets.get(line.target).map_or("", |t| t.id.as_str());
            let prefix = format!(" {id}: {PROMPT}  ");
            return Some((fit_tail(&prefix, &line.text, CURSOR, width), ACCENT));
        }
        if let Some((text, tone)) = &self.notice {
            return Some((cut_with_ellipsis(&format!(" {text}"), width), tone.style()));
        }
        if let Some(p) = self.pending.first() {
            let id = list.targets.get(p.target).map_or("", |t| t.id.as_str());
            let more = match self.pending.len() {
                1 => String::new(),
                n => format!(" (+{} more)", n - 1),
            };
            let text = format!(
                " {id}: {} → {} · applies at the next tick · not saved{more}",
                words(&p.from),
                words(&p.to)
            );
            return Some((cut_with_ellipsis(&text, width), WARN));
        }
        let ids = self.unsaved(list);
        (!ids.is_empty()).then(|| {
            let text = format!(" venues of {} changed · not saved", ids.join(", "));
            (cut_with_ellipsis(&text, width), SECONDARY)
        })
    }

    /// The legend while the line is open: the error, or the known venues
    /// and the keys.
    pub fn legend(&self, width: usize) -> Option<(String, Style)> {
        let line = self.line.as_ref()?;
        if let Some(error) = &line.error {
            return Some((cut_with_ellipsis(&format!(" ✗ {error}"), width), ERR));
        }
        let head = format!(" venues: {}", self.known.join(", "));
        let room = width.saturating_sub(cols(LINE_KEYS));
        Some((
            format!("{}{LINE_KEYS}", cut_with_ellipsis(&head, room)),
            SECONDARY,
        ))
    }
}

/// Why the venues of `t` cannot be edited, if they cannot.
fn unsuited(t: &Target) -> Option<String> {
    match t.kind {
        TargetKind::Address => Some(format!("{} is an address: it has no venues", t.id)),
        _ if !t.needs_market() => Some(format!(
            "{} reads no market metric: turn one on in the wizard first",
            t.id
        )),
        _ => None,
    }
}

/// What `u` did, in words: the venue that went again, and the one that is
/// back with no history.
fn undo_note(current: &[String], before: &[String]) -> String {
    let mut parts = Vec::new();
    for v in current.iter().filter(|v| !before.contains(v)) {
        parts.push(format!("{v} removed again"));
    }
    for v in before.iter().filter(|v| !current.contains(v)) {
        parts.push(format!("{v} is fresh (its history was dropped)"));
    }
    if parts.is_empty() {
        parts.push("the venues are in their old order".to_string());
    }
    format!("undone: {}", parts.join("; "))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `prefix`, the end of `text` that fits, and `suffix`, in `width` columns.
/// A text that does not fit loses its start, behind an ellipsis: the
/// cursor stays in view.
fn fit_tail(prefix: &str, text: &str, suffix: &str, width: usize) -> String {
    let room = width.saturating_sub(cols(prefix) + cols(suffix));
    if cols(text) <= room {
        return format!("{prefix}{text}{suffix}");
    }
    let ellipsis = scope::display::text::ELLIPSIS;
    let mut kept: Vec<char> = Vec::new();
    let mut used = cols(ellipsis);
    for c in text.chars().rev() {
        let w = cols(&c.to_string());
        if used + w > room {
            break;
        }
        used += w;
        kept.push(c);
    }
    let tail: String = kept.into_iter().rev().collect();
    format!("{prefix}{ellipsis}{tail}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::watch::{Preset, Rule};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn known() -> Vec<String> {
        ["binance", "okx", "weex", "mexc"]
            .map(String::from)
            .to_vec()
    }

    /// A list: a pair on weex and binance, a token with a holder rule only,
    /// an address.
    fn list() -> Watchlist {
        let mut w = Watchlist::new("pegs");
        let mut pair = Target::new(TargetKind::Pair, "USDN");
        pair.venues = vec!["weex".into(), "binance".into()];
        pair.preset = Some(Preset::MarketHealth);
        let mut token = Target::new(TargetKind::Token, "PEPE");
        token.rules = vec![Rule::parse("holders >= 10").unwrap()];
        let address = Target::new(TargetKind::Address, "0xabc");
        w.targets = vec![pair, token, address];
        w
    }

    fn ctx(list: &Watchlist) -> Ctx<'_> {
        Ctx {
            list,
            run: 3,
            running: true,
        }
    }

    fn editor(list: &Watchlist) -> VenueEditor {
        VenueEditor::new(list).with(known(), None)
    }

    fn type_line(e: &mut VenueEditor, list: &Watchlist, text: &str) {
        for c in text.chars() {
            e.line_key(&ctx(list), key(KeyCode::Char(c)));
        }
    }

    fn enter(e: &mut VenueEditor, list: &Watchlist) {
        e.line_key(&ctx(list), key(KeyCode::Enter));
    }

    fn last(e: &VenueEditor) -> &VenueRecord {
        e.records.last().unwrap()
    }

    /// Why: the user edits the line they see. The prefill is what makes
    /// one key and one line enough to add, remove or reorder.
    #[test]
    fn the_line_starts_with_the_venues_that_will_run() {
        let list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        assert_eq!(e.line.as_ref().unwrap().text, "weex binance");
        // After an edit that waits, the line shows what the user asked for.
        type_line(&mut e, &list, " okx");
        enter(&mut e, &list);
        e.open(&ctx(&list), Some(0));
        assert_eq!(e.line.as_ref().unwrap().text, "weex binance okx");
    }

    /// Why: `file.rs` rejects each of these lists. If the editor accepted
    /// one, a target would read nothing from any venue.
    #[test]
    fn a_list_that_reads_nothing_is_refused_and_the_list_stays() {
        let list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        for _ in 0..12 {
            e.line_key(&ctx(&list), key(KeyCode::Backspace));
        }
        enter(&mut e, &list);
        assert_eq!(e.legend(100).unwrap().0.trim(), "✗ a pair needs a venue");
        assert!(e.is_open(), "the line stays open with the message");
        assert!(!e.has_pending());
        assert_eq!(last(&e).outcome, outcome::REFUSED);
        assert_eq!(last(&e).line.as_deref(), Some(""), "the exact line");
        assert!(e.queue.take().is_empty(), "nothing reached the run");

        for (target, why) in [
            (1, "PEPE reads no market metric"),
            (2, "0xabc is an address"),
        ] {
            let mut e = editor(&list);
            e.open(&ctx(&list), Some(target));
            assert!(!e.is_open());
            assert!(e.notice.as_ref().unwrap().0.contains(why), "{:?}", e.notice);
            assert_eq!(last(&e).outcome, outcome::REFUSED);
        }

        let mut e = editor(&list);
        e.open(
            &Ctx {
                running: false,
                ..ctx(&list)
            },
            Some(0),
        );
        assert!(!e.is_open());
        assert_eq!(e.notice.as_ref().unwrap().0, RUN_ENDED_NOTE);
    }

    /// Why: a token with a market rule has the same floor as a pair.
    #[test]
    fn a_token_with_a_market_rule_needs_a_venue_too() {
        let mut list = list();
        list.targets[1].rules = vec![Rule::parse("spread_pct < 1").unwrap()];
        list.targets[1].venues = vec!["okx".into()];
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(1));
        for _ in 0..3 {
            e.line_key(&ctx(&list), key(KeyCode::Backspace));
        }
        enter(&mut e, &list);
        assert!(
            e.legend(100)
                .unwrap()
                .0
                .contains("a token with a market rule needs a venue")
        );
    }

    /// Why: the label is the key of the memory, the chart history and the
    /// CSV. A new spelling would start a "new" venue by mistake.
    #[test]
    fn a_changed_case_keeps_the_spelling_of_the_venue_in_the_list() {
        let list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        // The first word in another case, a new venue as the user spells it.
        e.line.as_mut().unwrap().text = "Weex binance OKX".into();
        enter(&mut e, &list);
        let change = e.queue.take();
        assert_eq!(change[0].venues, ["weex", "binance", "OKX"]);
    }

    /// Why: the user must be able to back out, and a back-out must leave no
    /// trace in the run.
    #[test]
    fn escape_leaves_the_queue_and_the_list_and_is_recorded() {
        let list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, " okx");
        e.line_key(&ctx(&list), key(KeyCode::Esc));
        assert!(!e.is_open() && !e.has_pending() && !e.can_undo());
        assert!(e.queue.take().is_empty());
        let r = last(&e);
        assert_eq!(
            (r.outcome, r.line.as_deref()),
            (outcome::CANCELLED, Some("weex binance okx"))
        );
        assert_eq!(e.records.len(), 1, "only the cancel");
    }

    /// Why: a later edit of one target before the tick must give one change
    /// (the last), and the first must not vanish from the trail.
    #[test]
    fn two_edits_before_the_tick_queue_one_change_and_record_both() {
        let list = list();
        let mut e = editor(&list);
        for extra in [" okx", " mexc"] {
            e.open(&ctx(&list), Some(0));
            type_line(&mut e, &list, extra);
            enter(&mut e, &list);
        }
        let queued = e.queue.take();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].venues, ["weex", "binance", "okx", "mexc"]);
        let outcomes: Vec<_> = e.records.iter().map(|r| r.outcome).collect();
        assert_eq!(
            outcomes,
            [outcome::QUEUED, outcome::SUPERSEDED, outcome::QUEUED]
        );
    }

    /// Why: the journal must say which line was typed, when it was queued
    /// and at which run it took effect. The two times differ by up to one
    /// tick.
    #[test]
    fn an_applied_edit_carries_the_line_the_queue_time_and_the_run() {
        let mut list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, " okx");
        enter(&mut e, &list);
        let change = e.queue.take().remove(0);
        list.targets[0].venues = change.venues.clone();
        e.applied(&ctx(&list), 4, &[change]);
        let r = last(&e);
        assert_eq!(r.outcome, outcome::APPLIED);
        assert_eq!(r.line.as_deref(), Some("weex binance okx"));
        assert_eq!(r.applied_at_run, Some(4));
        assert!(chrono::DateTime::parse_from_rfc3339(r.queued_at.as_deref().unwrap()).is_ok());
        assert_eq!((r.from.len(), r.to.len()), (2, 3));
        assert!(!e.has_pending());
    }

    /// Why: it pins what undo promises and what it does not. A restored
    /// venue is fresh, and the footer says so.
    #[test]
    fn undo_restores_the_list_and_says_that_the_venue_is_fresh() {
        let mut list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, " okx");
        enter(&mut e, &list);
        let change = e.queue.take().remove(0);
        list.targets[0].venues = change.venues.clone();
        e.applied(&ctx(&list), 4, &[change]);
        e.undo(&ctx(&list));
        let back = e.queue.take();
        assert_eq!(back[0].venues, ["weex", "binance"]);
        assert_eq!(e.notice.as_ref().unwrap().0, "undone: okx removed again");
        assert!(!e.can_undo(), "one level");
        assert_eq!(
            e.records
                .iter()
                .filter(|r| r.outcome == outcome::UNDONE)
                .count(),
            1
        );
        e.undo(&ctx(&list));
        assert_eq!(e.notice.as_ref().unwrap().0, NOTHING_TO_UNDO_NOTE);
    }

    /// Why: undo promises the old list, not the old memory. The venue that
    /// an edit removed comes back with no history, and the note says it.
    #[test]
    fn undo_of_a_removed_venue_brings_it_back_fresh() {
        let mut list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        e.line.as_mut().unwrap().text = "weex".into();
        enter(&mut e, &list);
        let change = e.queue.take().remove(0);
        list.targets[0].venues = change.venues.clone();
        e.applied(&ctx(&list), 4, &[change]);
        e.undo(&ctx(&list));
        assert_eq!(
            e.notice.as_ref().unwrap().0,
            "undone: binance is fresh (its history was dropped)"
        );
    }

    /// Why: a change that has not reached the run needs no change in the
    /// run: undo takes it out of the queue.
    #[test]
    fn undo_of_an_edit_that_waits_takes_it_back_and_nothing_reaches_the_run() {
        let list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, " okx");
        enter(&mut e, &list);
        e.undo(&ctx(&list));
        assert!(e.queue.take().is_empty());
        assert!(!e.has_pending());
        assert_eq!(
            e.notice.as_ref().unwrap().0,
            "undone: the change had not reached the run"
        );
        assert_eq!(last(&e).outcome, outcome::UNDONE);
    }

    /// Why: no silent skip. A change that the run did not take before it
    /// ended is in the trail as refused, with the reason.
    #[test]
    fn a_run_that_ends_refuses_the_waiting_edit_and_closes_the_line() {
        let list = list();
        let mut e = editor(&list);
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, " okx");
        enter(&mut e, &list);
        e.open(&ctx(&list), Some(0));
        e.ended(&Ctx {
            running: false,
            ..ctx(&list)
        });
        assert!(!e.is_open() && !e.has_pending());
        let reasons: Vec<_> = e
            .records
            .iter()
            .filter(|r| r.outcome == outcome::REFUSED || r.outcome == outcome::CANCELLED)
            .map(|r| r.reason.as_deref().unwrap())
            .collect();
        assert_eq!(reasons, [REASON_RUN_ENDED, REASON_RUN_ENDED]);
    }

    /// Why: a trial edit must not rewrite a saved watchlist, and a save
    /// must not erase a hand edit.
    #[test]
    fn a_save_writes_the_list_unless_the_file_changed_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pegs.yaml");
        let mut list = list();
        std::fs::write(
            &path,
            format!("# mine\n{}", scope::watch::file::to_text(&list)),
        )
        .unwrap();
        let file = ListFile::read(path.clone());
        let on_disk_before = std::fs::read_to_string(&path).unwrap();
        let mut e = VenueEditor::new(&list).with(known(), Some(file));
        // An edit alone does not write.
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, " okx");
        enter(&mut e, &list);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), on_disk_before);
        // While the edit waits, the save waits.
        e.save(&ctx(&list));
        assert_eq!(e.notice.as_ref().unwrap().0, SAVE_PENDING_NOTE);
        let change = e.queue.take().remove(0);
        list.targets[0].venues = change.venues.clone();
        e.applied(&ctx(&list), 4, &[change]);
        assert_eq!(
            e.summary(&list).as_deref(),
            Some("venues of USDN changed in the run and not saved")
        );

        // A hand edit during the run: nothing is written.
        std::fs::write(&path, "# edited by hand\n").unwrap();
        e.save(&ctx(&list));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# edited by hand\n"
        );
        let r = last(&e);
        assert_eq!(r.outcome, outcome::SAVE_REFUSED);
        assert!(
            r.reason
                .as_deref()
                .unwrap()
                .contains("pegs.yaml changed on disk")
        );
        assert_ne!(r.loaded_sha256, r.disk_sha256);
        assert_eq!(r.loaded_sha256.as_ref().map(String::len), Some(64));

        // Unchanged since the start: the file is exactly the list.
        std::fs::write(&path, &on_disk_before).unwrap();
        e.save(&ctx(&list));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            scope::watch::file::to_text(&list)
        );
        assert!(
            e.notice
                .as_ref()
                .unwrap()
                .0
                .contains("comments in the file are not kept")
        );
        assert_eq!(e.summary(&list), None, "saved");
        // A second save is a save of the file the first one wrote.
        e.save(&ctx(&list));
        assert_eq!(last(&e).outcome, outcome::SAVED);
        assert!(!e.notice.as_ref().unwrap().0.contains("comments"));
    }

    /// Why: a list with no file has no guard and goes where the wizard
    /// puts a list: the folder of the saved watchlists. The test sandbox
    /// keeps this off the real folder.
    #[test]
    fn a_list_with_no_file_saves_to_the_watchlist_folder() {
        let list = list();
        let mut e = editor(&list);
        e.save(&ctx(&list));
        let path = scope::watch::file::path_in(&scope::paths::watchlists_dir(), "pegs");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            scope::watch::file::to_text(&list)
        );
        assert_eq!(last(&e).outcome, outcome::SAVED);
    }

    /// Why: the input line and the legend must not wrap in a narrow pane;
    /// a long venue list is cut, and the cursor stays in view.
    #[test]
    fn a_long_line_and_a_long_legend_fit_the_width_with_an_ellipsis() {
        let list = list();
        let mut e = VenueEditor::new(&list)
            .with((0..30).map(|i| format!("venue-number-{i}")).collect(), None);
        e.open(&ctx(&list), Some(0));
        type_line(&mut e, &list, &" abcdefghij".repeat(10));
        let (note, _) = e.note(&list, 79).unwrap();
        assert!(cols(&note) <= 79, "{note}");
        assert!(
            note.starts_with(" USDN: venues  …") && note.ends_with("ghij█"),
            "{note}"
        );
        let (legend, _) = e.legend(79).unwrap();
        assert!(cols(&legend) <= 79, "{legend}");
        assert!(
            legend.contains('…') && legend.ends_with("Enter apply · Esc cancel"),
            "{legend}"
        );
    }

    /// Why: two parsers drift. The wizard's add line and the editor must
    /// give the same message for the same words; a change to one alone
    /// fails this test.
    #[test]
    fn the_wizard_and_the_editor_give_the_same_message_for_the_same_words() {
        use crate::cli::tui::wizard::WizardState;
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("nowhere", "unknown venue `nowhere`"),
            ("weex WEEX", "venue `WEEX` is listed twice"),
            ("", "a pair needs a venue"),
        ];
        for (words, expect) in cases {
            let mut w = WizardState::new(
                Watchlist::new("x"),
                Vec::new(),
                known(),
                dir.path().to_path_buf(),
            );
            w.key(key(KeyCode::Char('a')));
            for c in format!("pair USDN {words}").chars() {
                w.key(key(KeyCode::Char(c)));
            }
            w.key(key(KeyCode::Enter));
            let wizard = w.error.clone().unwrap();

            let list = list();
            let mut e = editor(&list);
            e.open(&ctx(&list), Some(0));
            e.line.as_mut().unwrap().text = words.to_string();
            enter(&mut e, &list);
            let editor_says = e.line.as_ref().unwrap().error.clone().unwrap();
            assert_eq!(wizard, editor_says, "{words:?}");
            assert!(wizard.contains(expect), "{wizard}");
        }
    }
}
