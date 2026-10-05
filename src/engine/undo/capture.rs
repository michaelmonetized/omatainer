//! Command classification resolves selected targets on the renderer, exactly
//! where MIDI, IPC and GUI commands become persistent edits.
use super::*;

#[derive(Clone, Copy)]
enum Target {
    Global,
    Sampler(usize),
    Track(u8),
    Gain(u8, u16),
    Deck(u8),
    Seek(u8),
    Clip(u8, u16),
    Media(u8),
    Slot(Rack, usize),
    Effect(Rack, usize),
}
struct Plan {
    target: Target,
    name: Name,
    key: u64,
}
impl Plan {
    fn get(rt: &RtEngine, c: &Command) -> Option<Self> {
        use Command::*;
        let (target, name, key) = match c {
            SetBpm(_) | NudgeBpm(_) | Tap(_) => (Target::Global, Name::Tempo, 1),
            Quant(_) | ToggleQuant => (Target::Global, Name::Quantization, 2),
            Metronome => (Target::Global, Name::Metronome, 3),
            Xfader(_) => (Target::Global, Name::Crossfader, 4),
            XfaderCurve(_) => (Target::Global, Name::CrossfaderContour, 7),
            Master(_) => (Target::Global, Name::Master, 5),
            CueMix(_) => (Target::Global, Name::CueMix, 6),
            FxWet { slot, .. } | FxSelect { slot } if *slot < 3 => {
                (Target::Global, Name::MasterEffect, 10 + *slot as u64)
            }
            SamplerBank(_) | SamplerInst(_) | SamplerOct(_) => (Target::Global, Name::Sampler, 20),
            SamplerEdit(edit) => (Target::Sampler(rt.sampler_edit_index(edit)?), Name::Sampler, 21),
            TrackGain { track, .. }
            | TrackPan { track, .. }
            | Mute { track }
            | Solo { track }
            | Arm { track }
                if (*track as usize) < rt.tracks.len() =>
            {
                (Target::Track(*track), Name::Track, 1000 + *track as u64)
            }
            ClipGain { track, scene, .. }
                if (*track as usize) < rt.tracks.len() && (*scene as usize) < rt.scene_fx.len() =>
            {
                (
                    Target::Gain(*track, *scene),
                    Name::ClipGain,
                    10000 + (*track as u64) * session::MAX_SCENES as u64 + *scene as u64,
                )
            }
            MidiEdit(request) => (Target::Clip(request.baseline.track, request.baseline.scene),
                Name::ClipNotes, 100000 + request.baseline.track as u64 * session::MAX_SCENES as u64 + request.baseline.scene as u64),
            SetNotes { track, scene, .. }
                if (*track as usize) < rt.tracks.len() && (*scene as usize) < rt.scene_fx.len() =>
            {
                (
                    Target::Clip(*track, *scene),
                    Name::ClipNotes,
                    100000 + (*track as u64) * session::MAX_SCENES as u64 + *scene as u64,
                )
            }
            ComposeArm { track, scene }
                if *track < rt.tracks.len()
                    && *scene < rt.scene_fx.len()
                    && rt.tracks[*track].clips[*scene].kind == ClipKind::Empty =>
            {
                (
                    Target::Clip(*track as u8, *scene as u16),
                    Name::ComposeClip,
                    100000 + (*track as u64) * session::MAX_SCENES as u64 + *scene as u64,
                )
            }
            DeckAudio { deck, .. } | DeckUnload { deck } => (
                Target::Media(*deck % 2),
                Name::LoadMedia,
                400 + (*deck % 2) as u64,
            ),
            DeckSeek { deck, .. } => (
                Target::Seek(*deck % 2),
                Name::DeckSeek,
                410 + (*deck % 2) as u64,
            ),
            DeckSync { deck }
            | DeckPitch { deck, .. }
            | DeckGain { deck, .. }
            | DeckEq { deck, .. }
            | DeckFilter { deck, .. }
            | DeckPfl { deck }
            | DeckLoop { deck, .. }
            | DeckLoopIn { deck }
            | DeckLoopOut { deck }
            | DeckVinyl { deck }
            | DeckKeylock { deck }
            | DeckLoopDouble { deck }
            | DeckLoopHalf { deck }
            | DeckReloop { deck }
            | DeckEqCut { deck, .. }
            | DeckEqSolo { deck, .. }
            | DeckPitchRange { deck } => (
                Target::Deck(*deck % 2),
                Name::Deck,
                420 + (*deck % 2) as u64,
            ),
            DeckCue { deck } if !rt.decks[(*deck % 2) as usize].playing => (
                Target::Deck(*deck % 2),
                Name::Deck,
                420 + (*deck % 2) as u64,
            ),
            DeckControl { deck, control: super::super::deck_controls::Control::Tap | super::super::deck_controls::Control::LoopToggle | super::super::deck_controls::Control::LoopSelect | super::super::deck_controls::Control::Reloop | super::super::deck_controls::Control::LoopShift { .. }, .. } => (
                Target::Deck(*deck), Name::Deck, 420 + u64::from(*deck),
            ),
            DeckHotCue { deck, pad, del }
                if *del || !rt.decks[(*deck % 2) as usize].hotcues[(*pad % 8) as usize].set =>
            {
                (
                    Target::Deck(*deck % 2),
                    Name::Deck,
                    420 + (*deck % 2) as u64,
                )
            }
            DeckGrid { deck, .. } => (Target::Deck(*deck), Name::Grid, 460 + *deck as u64),
            DeckSourceGain { deck, .. } => (Target::Deck(*deck), Name::Deck, 470 + *deck as u64),
            DeckCueStyle { deck, pad, .. } => (
                Target::Deck(*deck), Name::CueStyle, 440 + (*deck as u64) * 8 + *pad as u64,
            ),
            DeckMatch => {
                let other = if rt.xfader <= 0.5 { 1 } else { 0 };
                (Target::Seek(other), Name::DeckSeek, 410 + other as u64)
            }
            FxAdd(kind) => {
                let rack = Rack::selected(rt)?;
                let id = *fx::FxId::all().get(*kind as usize)?;
                if matches!(rack, Rack::Scene(_)) && !id.supports_scene() {
                    return None;
                }
                (
                    Target::Slot(rack, rack.get(rt).slots.len()),
                    Name::AddEffect,
                    500,
                )
            }
            FxToggle(index) | FxMix { slot: index, .. } | FxParam { slot: index, .. } => {
                let rack = Rack::selected(rt)?;
                if *index >= 128 {
                    return None;
                }
                rack.get(rt).slots.get(*index)?;
                (
                    Target::Effect(rack, *index),
                    Name::Effect,
                    1000000
                        + *index as u64
                        + match rack {
                            Rack::Track(t) => t as u64 * 128,
                            Rack::Scene(s) => (session::MAX_TRACKS as u64 + s as u64) * 128,
                        },
                )
            }
            _ => return None,
        };
        Some(Self { target, name, key })
    }
}

impl RtEngine {
    /// Capture an inverse before the first mutation. A rejected command still
    /// retires its owned payload on the worker, never at this callback boundary.
    pub(in crate::engine) fn history_before(&mut self, c: Command) -> Option<Command> {
        if let Command::FxAdd(index)=&c {
            if let Some(id)=fx::FxId::all().get(*index as usize) {
                let current=self.tracks.iter().map(|t|t.fx.slots.iter().map(fx::FxSlot::storage_bytes).sum::<usize>()).sum::<usize>()
                    + self.scene_fx.iter().map(|r|r.slots.iter().map(fx::FxSlot::storage_bytes).sum::<usize>()).sum::<usize>();
                if current.saturating_add(fx::FxSlot::required_storage(*id,self.sr))>session::MAX_PROCESSOR_BYTES {
                    self.undo.reject(Failure::ProcessorBudget);self.undo.retire_command(c);return None;
                }
            }
        }
        if let Command::SessionEdit(request) = c { self.history_session(request); return None; }
        if let Command::MidiImport(request) = c {
            self.history_midi_import(request); return None;
        }
        if let Command::MidiEdit(request) = &c {
            if !self.midi_edit_current(request) || !request.ack.claim() {
                self.undo.reject(Failure::Invalid); self.undo.retire_command(c); return None;
            }
            if request.unchanged() { request.ack.applied(); self.undo.retire_command(c); return None; }
        }
        if let Command::SamplerEdit(edit) = &c {
            if self.sampler_edit_index(edit).is_none() || !edit.ack.claim() {
                self.undo.reject(Failure::Invalid); self.undo.retire_command(c); return None;
            }
        }
        if let Command::DeckSourceGain { deck, gain, receipt, ack } = &c {
            let current = self.decks.get(*deck as usize).filter(|deck| deck.audio.is_some()
                && !deck.source_gain_active()
                && receipt.state() == load_receipt::State::Current
                && gain.level() == receipt.source_level()
                && deck.load_receipt.as_ref().is_some_and(|loaded| loaded.same_request(receipt)));
            let Some(deck) = current else { self.undo.reject(Failure::Invalid); self.undo.retire_command(c); return None; };
            if deck.source_gain == *gain { ack.applied(); self.undo.retire_command(c); return None; }
        }
        if let Command::DeckGrid { deck, grid, receipt, ack } = &c {
            let current = self.decks.get(*deck as usize).filter(|d| d.audio.is_some()
                && receipt.state() == load_receipt::State::Current
                && !receipt.grid_is_locked()
                && d.load_receipt.as_ref().is_some_and(|r| r.same_request(receipt)));
            let Some(deck) = current else {
                self.undo.reject(Failure::Invalid); self.undo.retire_command(c); return None;
            };
            if deck.grid == *grid { ack.applied(); self.undo.retire_command(c); return None; }
        }
        if let Command::DeckCueStyle { deck, pad, style, receipt } = &c {
            let current = self.decks.get(*deck as usize).filter(|d| {
                d.audio.is_some() && (*pad as usize) < HOTCUES
                    && d.hotcues[*pad as usize].set
                    && receipt.state() == load_receipt::State::Current
                    && d.load_receipt.as_ref().is_some_and(|r| r.same_request(receipt))
            });
            let Some(deck) = current else {
                self.undo.reject(Failure::Invalid);
                self.undo.retire_command(c);
                return None;
            };
            if deck.cue_styles[*pad as usize] == *style {
                self.undo.retire_command(c);
                return None;
            }
        }
        if !self.undo.enabled || self.undo.replaying {
            return Some(c);
        }
        if matches!(&c, Command::SetNotes { track, scene, .. }
            if *track as usize >= self.tracks.len() || *scene as usize >= self.scene_fx.len())
        {
            self.undo.retire_command(c);
            return None;
        }
        if matches!(&c, Command::FxAdd(_))
            && Rack::selected(self).is_some_and(|r| r.get(self).slots.len() >= 128)
        {
            self.history_reject(c, Failure::Effects);
            return None;
        }
        let Some(plan) = Plan::get(self, &c) else {
            return Some(c);
        };
        if !matches!(
            plan.target,
            Target::Clip(..) | Target::Media(..) | Target::Slot(..) | Target::Sampler(..)
        ) && self.undo.can_group(plan.key, self.frames_done)
        {
            self.undo.changed();
            return Some(c);
        }
        if matches!(&c,Command::DeckAudio {audio,..} if audio.name.len()>TEXT_LIMIT) {
            self.history_reject(c, Failure::Text);
            return None;
        }
        let estimate = match plan.target {
            Target::Global => self.conductor.as_ref().map_or(0, |c| c.bytes())
                + self.sampler_poly.offline.as_ref().map_or(0, |device| device.bytes()),
            Target::Sampler(index) => {
                let Command::SamplerEdit(edit) = &c else { unreachable!() };
                bank_bytes(&edit.bank) + self.sampler_banks.get(index).map_or(0, bank_bytes)
            }
            Target::Media(d) => {
                self.decks[d as usize]
                    .audio
                    .as_ref()
                    .map_or(0, |a| sample_bytes(a))
                    + match &c {
                        Command::DeckAudio { audio, .. }
                            if self.decks[d as usize]
                                .audio
                                .as_ref()
                                .is_none_or(|old| !Arc::ptr_eq(old, audio)) =>
                        {
                            sample_bytes(audio)
                        }
                        _ => 0,
                    }
                    + TEXT_LIMIT
            }
            Target::Clip(t, s) => {
                let old = &self.tracks[t as usize].clips[s as usize];
                if old.name.len() > TEXT_LIMIT {
                    self.history_reject(c, Failure::Text);
                    return None;
                }
                if old.notes.len() > NOTE_LIMIT
                    || old.notes.capacity() > NOTE_LIMIT
                    || matches!(&c,Command::SetNotes { notes,.. } if notes.len()>NOTE_LIMIT || notes.capacity()>NOTE_LIMIT)
                {
                    self.history_reject(c, Failure::Notes);
                    return None;
                }
                old.name.capacity().max(TEXT_LIMIT)
                    + 2 * NOTE_LIMIT * std::mem::size_of::<MidiNote>()
                    + old.audio.as_ref().map_or(0, |a| sample_bytes(a))
                    + old.lanes.as_ref().map_or(0, |l| l.bytes())
            }
            Target::Slot(..) => match &c {
                Command::FxAdd(kind) => {
                    fx::FxSlot::required_storage(fx::FxId::all()[*kind as usize], self.sr)
                }
                _ => 0,
            },
            _ => 0,
        };
        if let Err(reason) = self.undo.preflight(estimate) {
            self.history_reject(c, reason);
            return None;
        }
        let mut scratch = if matches!(plan.target, Target::Clip(..) | Target::Media(..)) {
            match self.undo.scratch.as_ref().unwrap().try_recv() {
                Ok(s) => Some(s),
                Err(_) => {
                    self.history_reject(c, Failure::Capacity);
                    return None;
                }
            }
        } else {
            None
        };
        let patch = match plan.target {
            Target::Sampler(index) => {
                let Command::SamplerEdit(edit) = &c else { unreachable!() };
                Patch::Sampler {
                    index, value: self.sampler_banks.get(index).cloned(), selected: self.sampler_bank,
                    original: self.sampler_banks.get(index).map(|bank| bank.data.clone()),
                    replacement: edit.bank.data.clone(),
                }
            }
            Target::Global => Patch::Global(Global::get(self)),
            Target::Track(t) => Patch::Track(t, TrackControls::get(&self.tracks[t as usize])),
            Target::Gain(t, s) => Patch::ClipGain {
                track: t,
                scene: s,
                gain: self.tracks[t as usize].clips[s as usize].gain,
            },
            Target::Deck(d) => Patch::Deck(d, DeckControls::get(&self.decks[d as usize])),
            Target::Seek(d) => {
                // Seeking updates cue/loop settings too. Both inverse values
                // belong to the same validated transaction.
                self.undo.begin(plan.name, plan.key, self.frames_done);
                self.undo
                    .append(Patch::Deck(d, DeckControls::get(&self.decks[d as usize])));
                Patch::Position {
                    deck: d,
                    position: self.decks[d as usize].pos,
                }
            }
            Target::Clip(t, s) => {
                let prepared = scratch.take().unwrap();
                let clip = &mut self.tracks[t as usize].clips[s as usize];
                let mut name = prepared.name;
                name.push_str(&clip.name);
                let name = std::mem::replace(&mut clip.name, name);
                let notes = std::mem::take(&mut clip.notes);
                Patch::Clip {
                    track: t,
                    scene: s,
                    value: Clip {
                        lanes: clip.lanes.clone(),
                        region: clip.region,
                        name,
                        notes,
                        kind: clip.kind,
                        bars: clip.bars,
                        gain: clip.gain,
                        audio: clip.audio.clone(),
                    },
                    spare_notes: prepared.notes,
                    reserved_midi_bytes: 0,
                }
            }
            Target::Media(d) => {
                let prepared = scratch.take().unwrap();
                let deck = &mut self.decks[d as usize];
                let patch = Patch::Media {
                    deck: d,
                    reserved_original: deck.audio.clone(),
                    audio: deck.audio.take(),
                    title: std::mem::replace(&mut deck.title, prepared.name),
                    bpm: deck.bpm,
                    position: deck.pos,
                    controls: DeckControls::get(deck),
                    receipt: PinnedReceipt::new(deck.load_receipt.clone()),
                    reserved_media: match &c {
                        Command::DeckAudio { audio, .. } => Some(audio.clone()),
                        _ => None,
                    },
                };
                self.undo.retire(
                    Retired::Notes(prepared.notes),
                    NOTE_LIMIT * std::mem::size_of::<MidiNote>(),
                );
                patch
            }
            Target::Slot(rack, index) => Patch::Slot {
                rack,
                index,
                slot: None,
                reserved_bytes: estimate,
                id: match &c {
                    Command::FxAdd(kind) => fx::FxId::all()[*kind as usize],
                    _ => unreachable!(),
                },
            },
            Target::Effect(rack, index) => Patch::Effect {
                rack,
                index,
                value: Effect::get(&rack.get(self).slots[index]),
            },
        };
        if !matches!(plan.target, Target::Seek(_)) {
            self.undo.begin(plan.name, plan.key, self.frames_done);
        }
        self.undo.append(patch);
        self.undo.recount();
        Some(c)
    }
    pub(super) fn history_reject(&mut self, c: Command, reason: Failure) {
        self.undo.reject(reason);
        super::super::midi_edit::reject_retired(&c);
        let bytes = command_bytes(&c);
        self.undo.retire(Retired::Command(c), bytes);
    }
}
pub(super) fn command_bytes(command: &Command) -> usize {
    match command {
        Command::SessionControl(scoped) => std::mem::size_of::<Command>() + command_bytes(&scoped.command),
        Command::SessionEdit(request) => request.bytes(),
        Command::MidiEdit(request) => request.bytes(),
        Command::MidiImport(request) => request.bytes(),
        Command::SamplerEdit(edit) => bank_bytes(&edit.bank),
        Command::ProviderPreview(request) => request.bytes(),
        Command::SamplerAudition(request) => request.bank.metadata_bytes() + request.bank.audio.iter().flatten().map(|sample| sample_bytes(sample)).sum::<usize>(),
        Command::SetNotes { notes, .. } => notes.capacity() * std::mem::size_of::<MidiNote>(),
        Command::DeckAudio { audio, .. }
        | Command::DeckDecoded { audio, .. }
        | Command::DeckLoadRequested {
            media: load_receipt::Media::Decoded { audio, .. },
            ..
        } => sample_bytes(audio),
        Command::Gesture { command, .. } => std::mem::size_of::<Command>() + command_bytes(command),
        _ => 0,
    }
}

pub(super) fn bank_bytes(bank: &sampler::Bank) -> usize {
    bank.data.metadata_bytes() + bank.data.audio.iter().flatten().map(|audio| sample_bytes(audio)).sum::<usize>()
}

impl RtEngine {
    pub(in crate::engine) fn history_session(&mut self, mut request: session::Request) {
        if !request.current(self) || !request.ack.claim() {
            request.ack.reject(); self.undo.reject(crate::engine::undo::Failure::Invalid);
            self.undo.retire_command(Command::SessionEdit(request)); return;
        }
        request.inverse.as_mut().unwrap().reserve(self);
        let inverse = request.inverse.as_ref().unwrap();
        if self.undo.enabled {
            let mut new_assets = 0usize;
            inverse.media_reservations(|pointer, _| { if self.undo.assets.binary_search_by_key(&pointer, |a| a.0).is_err() { new_assets += 1; } });
            let room = self.undo.assets.len().saturating_add(new_assets) <= self.undo.assets.capacity();
            if let Err(error) = if room { self.undo.preflight(request.bytes()) } else { Err(Failure::Budget) } {
                request.ack.reject(); self.undo.reject(error);
                self.undo.retire_command(Command::SessionEdit(request)); return;
            }
        }
        let mut inverse = request.inverse.take().unwrap();
        inverse.swap(self);
        if self.undo.enabled {
            self.undo.begin(crate::engine::undo::Name::Session, 2_000_000, self.frames_done);
            self.undo.append(crate::engine::undo::patch::Patch::Session(inverse)); self.undo.recount();
        } else { request.inverse = Some(inverse); }
        self.project.edited(); request.ack.applied();
        self.undo.retire_command(Command::SessionEdit(request));
    }
}
