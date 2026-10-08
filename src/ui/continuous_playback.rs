use super::*;
use crate::engine::deck_continue::{Lease,Request,Outcome};
use crate::library::{crates::CrateId,TrackId};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Panel {pub open:bool,decks:[Deck;DECKS]}
#[derive(Default)]
struct Deck {crate_id:Option<CrateId>,repeat:bool,run:Option<Run>,message:String,skipped:VecDeque<String>}
struct Run {
    lease:Arc<Lease>,members:Vec<TrackId>,cursor:usize,remaining:usize,safety:u64,
    watch:Option<Watch>,pending:Option<Pending>,
}

#[cfg(test)]
mod tests;
struct Watch {media:u64,end:u64,transport:u64}
enum Pending {Load {revision:u64,transport:u64,previous_media:u64},Start {request:Request,media:u64}}
impl Drop for Run {fn drop(&mut self){self.lease.disable();}}

impl App {
    /// End only one explicitly owned continuous-playback run.
    /// Takes its deck and visible reason; revokes automatic starts and cancels only its still-current pending load.
    pub(super) fn disable_continuous_playback(&mut self,deck:usize,reason:&str) {
        let Some(target)=self.continuous_playback.decks.get_mut(deck) else {return;};
        if let Some(run)=target.run.take() {
            run.lease.disable();
            if let Some(Pending::Load {revision,..})=run.pending.as_ref() {
                if self.load_revision[deck]==*revision {
                    if let Some(load)=&self.loads[deck] {if let Some(receipt)=&load.receipt {receipt.cancel_pending();}}
                    if let Some(loader)=&self.loader {let _=loader.invalidate(deck as u8);}
                }
            }
            target.message=reason.into();
        }
    }
    /// Capture manual crate order and arm one independent deck.
    /// Takes its deck; resumes a paused member or waits for the currently playing source's natural end.
    fn enable_continuous_playback(&mut self,deck:usize) {
        self.disable_continuous_playback(deck,"Continuous playback disabled");
        let state=&self.continuous_playback.decks[deck];
        let Some(node)=state.crate_id.as_ref().and_then(|id|self.library_metadata.catalog.crates.node(id)) else {self.continuous_playback.decks[deck].message="Choose an ordered crate first".into();return;};
        if node.is_smart()||node.members.is_empty() {self.continuous_playback.decks[deck].message="Continuous playback needs a nonempty manual ordered crate".into();return;}
        if self.project.committing()||!self.project.dialog_is_closed()||!self.engine.cmd.is_connected() {self.continuous_playback.decks[deck].message="Finish the current project decision before enabling continuous playback".into();return;}
        let snapshot=self.engine.snapshot();let source=&snapshot.decks[deck];let members=node.members.clone();
        let current=self.loads[deck].as_ref().and_then(|load|load.selection.as_ref()).and_then(|selection|self.library_metadata.catalog.track(&selection.source)).map(|track|&track.id);
        let current_position=current.and_then(|id|members.iter().position(|member|member==id));
        let cursor=current_position.map_or(0,|position|position+1);
        let mut run=Run {lease:Lease::new(),remaining:members.len(),members,cursor,safety:self.engine.cmd.performance().safety_epoch(),watch:None,pending:None};
        self.continuous_playback.decks[deck].skipped.clear();
        if source.playing {run.watch=Some(Watch {media:source.media_key,end:source.natural_end,transport:source.transport_generation});}
        else if current_position.is_some()&&source.frames>0.0&&source.media_key!=0 {if !self.start_continuous_source(deck,&mut run,source) {return;}}
        self.continuous_playback.decks[deck].message="Continuous playback enabled · captured crate order".into();
        self.continuous_playback.decks[deck].run=Some(run);
    }
    /// Queue one source-qualified start with immediate disable ownership.
    /// Takes its deck, run and current applied source; retains renderer acknowledgement so even sub-frame tracks cannot lose their end event.
    fn start_continuous_source(&mut self,deck:usize,run:&mut Run,source:&crate::engine::DeckSnap)->bool {
        let request=match Request::new(deck as u8,source,run.safety,run.lease.clone()) {Ok(request)=>request,Err(error)=>{self.continuous_playback.decks[deck].message=error;return false;}};
        if !self.submit(Command::DeckContinue(request.clone())) {self.continuous_playback.decks[deck].message="Automatic start was refused; playback remains manually controlled".into();return false;}
        run.pending=Some(Pending::Start {request,media:source.media_key});true
    }
    /// Report a failed source while keeping skip storage bounded.
    /// Takes its deck and failure; retains the sixteen most recent skipped items.
    fn continuous_skip(&mut self,deck:usize,message:String) {
        let state=&mut self.continuous_playback.decks[deck];state.message=message.clone();
        if state.skipped.len()==16 {state.skipped.pop_front();}state.skipped.push_back(message);
    }
    /// Request the next source through the ordinary protected decoder.
    /// Takes its deck and captured order; performs no file reads and admits at most one load per frame.
    fn load_continuous_next(&mut self,deck:usize,run:&mut Run)->bool {
        if run.remaining==0 {self.continuous_playback.decks[deck].message="No available tracks remain in this crate pass".into();return false;}
        if run.cursor>=run.members.len() {
            if !self.continuous_playback.decks[deck].repeat {self.continuous_playback.decks[deck].message="Ordered crate finished".into();return false;}run.cursor=0;
        }
        let rows=self.library_metadata.collection_rows();
        if !rows.is_for(&self.library,&self.library_metadata.catalog) {return true;}
        let id=&run.members[run.cursor];run.cursor+=1;run.remaining-=1;
        let selection=rows.track_index(id,&self.library_metadata.catalog).map(|index|&self.library_metadata.catalog.tracks[index]).map(|track| {
            let version=&track.versions[track.current];Selection {source:track.source.clone(),title:version.metadata.title.clone(),fingerprint:version.fingerprint}
        });
        let Some(selection)=selection else {self.continuous_skip(deck,format!("Skipped unavailable crate member {}",id.0));return true;};
        if matches!(selection.source,LibSource::Provider {..}) {self.continuous_skip(deck,format!("Skipped {}: provider playback is unavailable locally",selection.title));return true;}
        let before=self.load_revision[deck];let transport=self.snap.decks[deck].transport_generation;
        self.load_source_approved(deck as u8,Some(&selection),None);
        if self.load_revision[deck]==before {self.continuous_playback.decks[deck].message="Protected deck load was refused; continuous playback stopped".into();return false;}
        run.pending=Some(Pending::Load {revision:self.load_revision[deck],transport,previous_media:self.snap.decks[deck].media_key});true
    }
    /// Follow acknowledged starts and source-qualified natural ends.
    /// Takes current native state; preserves other decks and stops automatic work after manual ownership, project or safety changes.
    pub(super) fn poll_continuous_playback(&mut self) {
        for deck in 0..DECKS {
            let Some(mut run)=self.continuous_playback.decks[deck].run.take() else {continue;};
            let source=self.snap.decks[deck].clone();
            let mut keep=run.lease.enabled()&&run.safety==self.engine.cmd.performance().safety_epoch()&&!self.project.committing()&&self.project.dialog_is_closed()&&self.engine.cmd.is_connected();
            if !keep {self.continuous_playback.decks[deck].run=Some(run);self.disable_continuous_playback(deck,"Project, audio or safety state changed; continuous playback stopped");continue;}
            if let Some(pending)=run.pending.take() {
                match pending {
                    Pending::Load {revision,transport,previous_media}=>{
                        if self.load_revision[deck]!=revision||source.transport_generation!=transport {self.continuous_playback.decks[deck].message="Manual deck control ended continuous playback".into();run.pending=Some(Pending::Load {revision,transport,previous_media});keep=false;}
                        else if let Some(load)=&self.loads[deck] {
                            let owned_applied=load.receipt.as_ref().is_some_and(|receipt|receipt.state()==crate::engine::load_receipt::State::Current&&receipt.history_key()==source.media_key);
                            if source.media_key!=previous_media&&!owned_applied {self.continuous_playback.decks[deck].message="Manual source replacement ended continuous playback".into();run.pending=Some(Pending::Load {revision,transport,previous_media});keep=false;}
                            else {
                            match &load.phase {
                                load_status::Phase::Loading|load_status::Phase::Queued=>run.pending=Some(Pending::Load {revision,transport,previous_media}),
                                load_status::Phase::Loaded if owned_applied=>keep=self.start_continuous_source(deck,&mut run,&source),
                                load_status::Phase::Loaded if source.media_key==previous_media&&load.receipt.as_ref().is_some_and(|receipt|receipt.state()==crate::engine::load_receipt::State::Current)=>run.pending=Some(Pending::Load {revision,transport,previous_media}),
                                load_status::Phase::Failed(error) if load.receipt.is_none()=>self.continuous_skip(deck,format!("Skipped {}: {error}",load.selection.as_ref().map_or("track",|selection|&selection.title))),
                                _=>{self.continuous_playback.decks[deck].message="Load ownership changed; continuous playback stopped".into();run.pending=Some(Pending::Load {revision,transport,previous_media});keep=false;},
                            }
                            }
                        } else {self.continuous_playback.decks[deck].message="Pending load was dismissed; continuous playback stopped".into();keep=false;}
                    }
                    Pending::Start {request,media}=>match request.outcome() {
                        Outcome::Pending=>run.pending=Some(Pending::Start {request,media}),
                        Outcome::Started {end,transport}=>{run.remaining=run.members.len();run.watch=Some(Watch {media,end,transport});},
                        Outcome::Refused=>{self.continuous_playback.decks[deck].message="Manual control or start protection ended continuous playback".into();keep=false;},
                    }
                }
            }
            if keep&&run.pending.is_none() {
                if let Some(watch)=&run.watch {
                    if source.media_key!=watch.media||source.transport_generation!=watch.transport {self.continuous_playback.decks[deck].message="Manual deck control ended continuous playback".into();keep=false;}
                    else if source.natural_end!=watch.end&&source.end_media_key==watch.media {run.watch=None;}
                }
                if keep&&run.watch.is_none() {keep=self.load_continuous_next(deck,&mut run);}
            }
            self.continuous_playback.decks[deck].run=Some(run);
            if !keep {let message=self.continuous_playback.decks[deck].message.clone();self.disable_continuous_playback(deck,&message);}
        }
    }
    /// Configure explicit independent continuous playback in the native app.
    /// Takes the GUI context; exposes captured manual order, next track, finite/repeat policy and accessible enable/disable controls.
    pub(super) fn continuous_playback_ui(&mut self,ctx:&egui::Context) {
        if !self.continuous_playback.open {return;}keyboard::block_for_dialog(ctx);
        let mut open=true;let mut actions=Vec::new();
        egui::Window::new("Continuous playback").id(egui::Id::new("continuous-playback")).open(&mut open).resizable(false).show(ctx,|ui| {
            ui.label("Enable one deck to play captured manual crate order. A playing track finishes first; a paused crate member resumes. Otherwise the first member loads.");
            ui.label("Manual track or transport changes stop automatic playback. Reopening starts disabled.");
            for deck in 0..DECKS {ui.push_id(deck,|ui| {
                let label=(b'A'+deck as u8) as char;ui.separator();ui.heading(format!("Deck {label}"));
                let state=&mut self.continuous_playback.decks[deck];let enabled=state.run.is_some();
                let name=state.crate_id.as_ref().and_then(|id|self.library_metadata.catalog.crates.node(id)).map_or("Choose a crate",|node|node.name.as_str());
                let response=egui::ComboBox::from_id_salt("continuous-crate").selected_text(name).show_ui(ui,|ui| {
                    for node in self.library_metadata.catalog.crates.nodes().iter().filter(|node|!node.is_smart()) {
                        if ui.selectable_value(&mut state.crate_id,Some(node.id.clone()),&node.name).changed() {actions.push((deck,0));}
                    }
                }).response;
                response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::ComboBox,true,format!("Deck {label}: Continuous playback crate")));accessibility::focus(ui,&response);help::annotate(ui,&response,HelpControl::ContinuousPlayback);
                let response=ui.checkbox(&mut state.repeat,"Repeat ordered crate");response.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::Checkbox,true,state.repeat,format!("Deck {label}: Repeat ordered crate")));help::annotate(ui,&response,HelpControl::ContinuousPlayback);
                let mut active=enabled;let response=ui.checkbox(&mut active,"Enable continuous playback");response.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::Checkbox,true,active,format!("Deck {label}: Enable continuous playback")));help::annotate(ui,&response,HelpControl::ContinuousPlayback);
                if response.changed(){actions.push((deck,if active {1} else {0}));}
                let next_id=if let Some(run)=&state.run {run.members.get(if run.cursor>=run.members.len()&&state.repeat {0} else {run.cursor})} else {state.crate_id.as_ref().and_then(|id|self.library_metadata.catalog.crates.node(id)).and_then(|node| {
                    let current=self.loads[deck].as_ref().and_then(|load|load.selection.as_ref()).and_then(|selection|self.library_metadata.catalog.track(&selection.source)).map(|track|&track.id);
                    let next=current.and_then(|id|node.members.iter().position(|member|member==id)).map_or(0,|position|position+1);node.members.get(if next>=node.members.len()&&state.repeat {0} else {next})
                })};
                let next=next_id.map_or("End of crate",|id|self.library_metadata.collection_rows().track_index(id,&self.library_metadata.catalog).map(|index|self.library_metadata.catalog.tracks[index].versions[self.library_metadata.catalog.tracks[index].current].metadata.title.as_str()).unwrap_or("Preparing next track…"));
                ui.label(format!("Next: {next}"));ui.label(&state.message);
                if !state.skipped.is_empty(){egui::CollapsingHeader::new("Skipped tracks").show(ui,|ui|for message in &state.skipped {ui.label(message);});}
            });}
        });
        self.continuous_playback.open=open;
        for (deck,action) in actions {if action==1 {self.enable_continuous_playback(deck);} else {self.disable_continuous_playback(deck,"Continuous playback disabled");}}
    }
}
