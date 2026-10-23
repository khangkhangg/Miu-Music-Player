//! The party queue (P.1): guests add songs from the library or videos, everyone votes, the most
//! votes play first. The DJ can require approval, remove anything, and close the queue. The host
//! (Miu) holds the one true queue; guests (on the Wi-Fi page or through the relay) send actions.

use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The DJ: no limit, no approval needed.
pub const HOST: &str = "host";
/// A song that played this recently cannot be added again yet.
const RECENT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    /// A song from the DJ's library (mixed by Auto DJ).
    #[serde(rename_all = "camelCase")]
    Song { song_id: i64, title: String, artist: String },
    /// A YouTube video, played in YouTube's own player on the party screen (nothing downloaded).
    #[serde(rename_all = "camelCase")]
    Video { video_id: String, title: String },
}

impl Item {
    fn same(&self, other: &Item) -> bool {
        match (self, other) {
            (Item::Song { song_id: a, .. }, Item::Song { song_id: b, .. }) => a == b,
            (Item::Video { video_id: a, .. }, Item::Video { video_id: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub item: Item,
    pub added_by: String,
    pub added_name: String,
    seq: u64,
    pub votes: BTreeSet<String>,
    pub approved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Guests may add.
    pub open: bool,
    /// Guest additions wait for the DJ.
    pub approval: bool,
    /// How many of their own a guest may have waiting to play.
    pub per_guest: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { open: true, approval: false, per_guest: 3 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    Closed,
    Limit,
    JustPlayed,
}

#[derive(Debug, Default)]
pub struct Queue {
    entries: Vec<Entry>,
    now: Option<Entry>,
    recent: VecDeque<Item>,
    next_id: u64,
    version: u64,
    pub settings: Settings,
    /// When guests added or voted lately (for the crowd's heat).
    activity: VecDeque<Instant>,
}

/// One queue entry as a guest sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub id: u64,
    #[serde(flatten)]
    pub item: Item,
    pub added_name: String,
    pub votes: usize,
    pub voted: bool,
    pub mine: bool,
}

/// Everything a guest's page shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestView {
    pub version: u64,
    pub now: Option<EntryView>,
    pub queue: Vec<EntryView>,
    /// The guest's own additions waiting for the DJ.
    pub waiting_mine: usize,
    pub approval: bool,
    pub open: bool,
}

impl Queue {
    fn changed(&mut self) {
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// How busy the guests are: adds and votes in the last ten minutes, 20 of them is full heat.
    pub fn heat(&self, now: Instant) -> f32 {
        let recent = self.activity.iter().filter(|&&t| now.saturating_duration_since(t) < Duration::from_secs(600)).count();
        (recent as f32 / 20.0).min(1.0)
    }

    fn stir(&mut self, guest: &str) {
        if guest != HOST {
            if self.activity.len() >= 64 {
                self.activity.pop_front();
            }
            self.activity.push_back(Instant::now());
        }
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Adds `item` for `guest` (or votes for it when it is already queued). Returns the entry.
    pub fn add(&mut self, guest: &str, name: &str, item: Item) -> Result<u64, Refusal> {
        let host = guest == HOST;
        if !host && !self.settings.open {
            return Err(Refusal::Closed);
        }
        if let Some(e) = self.entries.iter_mut().find(|e| e.item.same(&item)) {
            e.votes.insert(guest.to_string());
            let id = e.id;
            self.stir(guest);
            self.changed();
            return Ok(id);
        }
        if self.recent.iter().any(|r| r.same(&item)) || self.now.as_ref().is_some_and(|n| n.item.same(&item)) {
            return Err(Refusal::JustPlayed);
        }
        if !host && self.entries.iter().filter(|e| e.added_by == guest).count() >= self.settings.per_guest {
            return Err(Refusal::Limit);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.entries.push(Entry {
            id,
            item,
            added_by: guest.to_string(),
            added_name: name.to_string(),
            seq: id,
            votes: BTreeSet::from([guest.to_string()]),
            approved: host || !self.settings.approval,
        });
        self.stir(guest);
        self.changed();
        Ok(id)
    }

    /// `on`: vote for the entry; off: take the vote back.
    pub fn vote(&mut self, guest: &str, id: u64, on: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            let changed = if on { e.votes.insert(guest.to_string()) } else { e.votes.remove(guest) };
            if changed {
                if on {
                    self.stir(guest);
                }
                self.changed();
            }
        }
    }

    /// Lets a waiting addition in: it joins the queue now (not from when it was asked for).
    pub fn approve(&mut self, id: u64) {
        self.next_id += 1;
        let seq = self.next_id;
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id && !e.approved) {
            e.approved = true;
            e.seq = seq;
            self.changed();
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
        self.changed();
    }

    /// The approved entries in play order: most votes first, then first come.
    pub fn ordered(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.entries.iter().filter(|e| e.approved).collect();
        v.sort_by(|a, b| b.votes.len().cmp(&a.votes.len()).then(a.seq.cmp(&b.seq)));
        v
    }

    /// Guest additions waiting for the DJ's approval, oldest first.
    pub fn waiting(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| !e.approved).collect()
    }

    pub fn now(&self) -> Option<&Entry> {
        self.now.as_ref()
    }

    /// Takes the top entry off the queue: it is what plays now.
    pub fn next(&mut self) -> Option<Entry> {
        let id = self.ordered().first()?.id;
        let i = self.entries.iter().position(|e| e.id == id)?;
        let e = self.entries.remove(i);
        if let Some(prev) = self.now.replace(e.clone()) {
            self.recent.push_front(prev.item);
            self.recent.truncate(RECENT);
        }
        self.changed();
        Some(e)
    }

    /// The entry that plays next (left in the queue until it starts).
    pub fn peek(&self) -> Option<&Entry> {
        self.ordered().into_iter().next()
    }

    /// `item` went on air and becomes what plays now: if it is queued it leaves the queue; if it
    /// came from outside (the DJ, Auto DJ's own pick) guests see it playing all the same.
    pub fn start_item(&mut self, item: &Item) {
        let started = match self.entries.iter().position(|e| e.item.same(item)) {
            Some(i) => self.entries.remove(i),
            None => {
                self.next_id += 1;
                Entry { id: self.next_id, item: item.clone(), added_by: HOST.into(), added_name: String::new(), seq: self.next_id, votes: BTreeSet::new(), approved: true }
            }
        };
        let started = Some(started);
        if let Some(prev) = std::mem::replace(&mut self.now, started) {
            self.recent.push_front(prev.item);
            self.recent.truncate(RECENT);
        }
        self.changed();
    }

    /// What plays now came from outside the queue (the DJ, Auto DJ's own pick): nothing from it.
    pub fn clear_now(&mut self) {
        if let Some(prev) = self.now.take() {
            self.recent.push_front(prev.item);
            self.recent.truncate(RECENT);
            self.changed();
        }
    }

    fn entry_view(e: &Entry, guest: &str) -> EntryView {
        EntryView {
            id: e.id,
            item: e.item.clone(),
            added_name: e.added_name.clone(),
            votes: e.votes.len(),
            voted: e.votes.contains(guest),
            mine: e.added_by == guest,
        }
    }

    pub fn view(&self, guest: &str) -> GuestView {
        GuestView {
            version: self.version,
            now: self.now.as_ref().map(|e| Self::entry_view(e, guest)),
            queue: self.ordered().into_iter().map(|e| Self::entry_view(e, guest)).collect(),
            waiting_mine: self.entries.iter().filter(|e| !e.approved && e.added_by == guest).count(),
            approval: self.settings.approval,
            open: self.settings.open,
        }
    }

    /// The queue for the relay site, which works out each guest's view from it (see `view`).
    pub fn relay_view(&self) -> RelayView {
        let entry = |e: &Entry| RelayEntry {
            id: e.id,
            item: e.item.clone(),
            added_by: e.added_by.clone(),
            added_name: e.added_name.clone(),
            voters: e.votes.iter().cloned().collect(),
        };
        RelayView {
            version: self.version,
            now: self.now.as_ref().map(entry),
            queue: self.ordered().into_iter().map(entry).collect(),
            waiting: self.entries.iter().filter(|e| !e.approved).map(|e| e.added_by.clone()).collect(),
            approval: self.settings.approval,
            open: self.settings.open,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayEntry {
    pub id: u64,
    #[serde(flatten)]
    pub item: Item,
    pub added_by: String,
    pub added_name: String,
    pub voters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayView {
    pub version: u64,
    pub now: Option<RelayEntry>,
    pub queue: Vec<RelayEntry>,
    /// Who has songs waiting for the DJ's approval (one per song).
    pub waiting: Vec<String>,
    pub approval: bool,
    pub open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: i64) -> Item {
        Item::Song { song_id: id, title: format!("Song {id}"), artist: "Artist".into() }
    }

    fn video(id: &str) -> Item {
        Item::Video { video_id: id.into(), title: format!("Video {id}") }
    }

    fn ids(q: &Queue) -> Vec<u64> {
        q.ordered().iter().map(|e| e.id).collect()
    }

    #[test]
    fn the_crowd_heats_up_as_guests_add_and_vote() {
        let mut q = Queue::default();
        let now = std::time::Instant::now();
        assert_eq!(q.heat(now), 0.0);
        let a = q.add("ann", "Ann", song(1)).unwrap();
        q.add("bob", "Bob", song(2)).unwrap();
        q.vote("cat", a, true);
        q.add(HOST, "", song(3)).unwrap();
        assert!((q.heat(now) - 3.0 / 20.0).abs() < 1e-6, "guests only: {}", q.heat(now));
        for i in 0..30 {
            q.vote(&format!("g{i}"), a, true);
        }
        assert_eq!(q.heat(now), 1.0, "capped");
        assert_eq!(q.heat(now + std::time::Duration::from_secs(11 * 60)), 0.0, "cools after ten minutes");
    }

    #[test]
    fn the_most_votes_play_first_and_ties_keep_the_order_they_came_in() {
        let mut q = Queue::default();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        let b = q.add("bob", "Bob", song(2)).unwrap();
        let c = q.add("cat", "Cat", video("dQw4w9WgXcQ")).unwrap();
        assert_eq!(ids(&q), vec![a, b, c], "one vote each (their own): first come first");
        q.vote("ann", c, true);
        q.vote("dan", c, true);
        q.vote("dan", b, true);
        assert_eq!(ids(&q), vec![c, b, a]);
        q.vote("dan", c, false);
        q.vote("dan", c, false);
        assert_eq!(q.get(c).unwrap().votes.len(), 2, "a vote can be taken back once");
        q.vote("ann", c, true);
        assert_eq!(q.get(c).unwrap().votes.len(), 2, "one vote per guest");
    }

    #[test]
    fn adding_what_is_already_queued_is_a_vote_for_it() {
        let mut q = Queue::default();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        assert_eq!(q.add("bob", "Bob", song(1)), Ok(a));
        assert_eq!(q.get(a).unwrap().votes.len(), 2);
        assert_eq!(q.ordered().len(), 1);
    }

    #[test]
    fn each_guest_has_a_limit_but_the_dj_does_not() {
        let mut q = Queue::default();
        q.settings.per_guest = 2;
        q.add("ann", "Ann", song(1)).unwrap();
        q.add("ann", "Ann", song(2)).unwrap();
        assert_eq!(q.add("ann", "Ann", song(3)), Err(Refusal::Limit));
        for i in 10..20 {
            q.add(HOST, "DJ", song(i)).unwrap();
        }
        // Once one of Ann's has played she can add again.
        let next = q.next().unwrap();
        assert_eq!(next.added_by, "ann");
        assert!(q.add("ann", "Ann", song(3)).is_ok());
    }

    #[test]
    fn with_approval_on_guest_songs_wait_for_the_dj() {
        let mut q = Queue::default();
        q.settings.approval = true;
        let a = q.add("ann", "Ann", song(1)).unwrap();
        let h = q.add(HOST, "DJ", song(2)).unwrap();
        assert_eq!(ids(&q), vec![h], "the DJ\x27s own go straight in");
        assert_eq!(q.waiting().iter().map(|e| e.id).collect::<Vec<_>>(), vec![a]);
        q.approve(a);
        assert_eq!(ids(&q), vec![h, a]);
        assert!(q.waiting().is_empty());
    }

    #[test]
    fn the_dj_removes_and_closes() {
        let mut q = Queue::default();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        q.remove(a);
        assert!(q.ordered().is_empty());
        q.settings.open = false;
        assert_eq!(q.add("bob", "Bob", song(2)), Err(Refusal::Closed));
        assert!(q.add(HOST, "DJ", song(2)).is_ok(), "the DJ can always add");
    }

    #[test]
    fn next_takes_the_top_and_remembers_what_played() {
        let mut q = Queue::default();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        let b = q.add("bob", "Bob", song(2)).unwrap();
        q.vote("cat", b, true);
        assert_eq!(q.next().map(|e| e.id), Some(b));
        assert_eq!(q.now().map(|e| e.id), Some(b));
        assert_eq!(ids(&q), vec![a]);
        assert_eq!(q.add("dan", "Dan", song(2)).map(|_| ()), Err(Refusal::JustPlayed), "not again right away");
        assert_eq!(q.next().map(|e| e.id), Some(a));
        assert!(q.next().is_none());
    }

    #[test]
    fn auto_dj_loads_the_top_early_and_starts_it_when_it_plays() {
        let mut q = Queue::default();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        let b = q.add("bob", "Bob", song(2)).unwrap();
        assert_eq!(q.peek().map(|e| e.id), Some(a), "the top, still in the queue");
        assert_eq!(q.ordered().len(), 2);
        q.vote("cat", b, true);
        q.start_item(&song(1));
        assert_eq!(q.now().map(|e| e.id), Some(a), "the loaded song plays even if votes moved");
        assert_eq!(q.ordered().iter().map(|e| e.id).collect::<Vec<_>>(), vec![b]);
        q.start_item(&song(7));
        let now = q.now().expect("a song from outside the queue (Auto DJ's own pick) still shows as playing");
        assert_eq!((now.item.clone(), now.added_by.as_str(), now.votes.len()), (song(7), HOST, 0));
        assert_eq!(q.ordered().len(), 1, "and the queue is left alone");
        assert_eq!(q.add("ann", "Ann", song(7)), Err(Refusal::JustPlayed), "it is playing: not again yet");
    }

    #[test]
    fn every_change_moves_the_version_on() {
        let mut q = Queue::default();
        let v0 = q.version();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        let v1 = q.version();
        q.vote("bob", a, true);
        let v2 = q.version();
        assert!(v0 < v1 && v1 < v2);
    }

    #[test]
    fn what_a_guest_sees() {
        let mut q = Queue::default();
        let a = q.add("ann", "Ann", song(1)).unwrap();
        q.add("bob", "Bob", video("abc")).unwrap();
        q.settings.approval = true;
        q.add("ann", "Ann", song(3)).unwrap();
        let v = q.view("ann");
        assert_eq!(v.queue.len(), 2);
        assert!(v.queue[0].mine && v.queue[0].voted && v.queue[0].votes == 1);
        assert!(!v.queue[1].mine && !v.queue[1].voted);
        assert_eq!(v.waiting_mine, 1, "her song waiting for the DJ");
        assert!(v.approval);
        let _ = a;
    }

    #[test]
    fn the_relay_gets_enough_to_show_each_guest_their_view() {
        let mut q = Queue::default();
        q.add("ann", "Ann", song(1)).unwrap();
        q.vote("bob", 1, true);
        q.settings.approval = true;
        q.add("cat", "Cat", video("abc")).unwrap();
        let r = serde_json::to_value(q.relay_view()).unwrap();
        assert_eq!(r["version"], q.version());
        assert_eq!(r["queue"][0]["addedBy"], "ann");
        assert_eq!(r["queue"][0]["voters"], serde_json::json!(["ann", "bob"]));
        assert_eq!(r["queue"][0]["title"], "Song 1");
        assert_eq!(r["waiting"], serde_json::json!(["cat"]), "only who is waiting, not what");
        assert_eq!(r["approval"], true);
        assert!(r["now"].is_null());
    }
}
