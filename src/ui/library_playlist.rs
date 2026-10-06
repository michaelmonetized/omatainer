use super::*;
use crate::playlist_import::{Input, Mapping, Review};
use library_metadata::CollectionAction;
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Import {
    pub open: bool,
    path: String,
    from: String,
    to: String,
    pub review: Option<(u64,Arc<Review>)>,
    selected: BTreeSet<usize>,
    allow_excluded: bool,
    generation: u64,
    pub message: String,
}
impl Import {
    pub fn accept_review(&mut self,revision:u64,review:Arc<Review>)->Result<(),String> {
        self.review=None;
        self.generation=self.generation.checked_add(1).ok_or("Playlist review identity exhausted; restart the app before importing")?;
        self.selected=review.playlists.iter().enumerate().filter(|(_,p)|p.rows.iter().any(|&i|review.rows[i].ready())).map(|(i,_)|i).collect();
        self.review=Some((revision,review));self.allow_excluded=false;Ok(())
    }
}
impl App {
    fn review_playlist(&mut self) {
        let state=&self.library_playlist;
        if state.from.is_empty()!=state.to.is_empty() {self.library_playlist.message="Supply both path-mapping fields, or leave both empty.".into();return;}
        let input=Input{path:PathBuf::from(&state.path),mapping:(!state.from.is_empty()).then(||Mapping{from:state.from.clone(),to:PathBuf::from(&state.to)})};
        self.library_playlist.review=None;
        self.submit_crate_edit(self.library_metadata.catalog.crates.revision(),CollectionAction::ReviewPlaylist(input));
        self.library_playlist.message=self.library_crates.message.clone();
    }
    fn import_reviewed_playlists(&mut self) {
        let state=&self.library_playlist;let Some((revision,review))=&state.review else{return};
        let excluded=state.selected.iter().flat_map(|&i|review.playlists[i].rows.iter()).any(|&i|!review.rows[i].ready());
        if excluded && !state.allow_excluded {self.library_playlist.message="Review the reported failures and explicitly allow their exclusion.".into();return;}
        let action=CollectionAction::ImportPlaylist{review:review.clone(),selected:state.selected.iter().copied().collect()};
        self.submit_crate_edit(*revision,action);self.library_playlist.message=self.library_crates.message.clone();
    }
    pub(super) fn playlist_import_ui(&mut self,ctx:&egui::Context) {
        if !self.library_playlist.open {return;}
        let mut open=true;
        egui::Window::new("Import playlists").id(egui::Id::new("playlist-import-window")).open(&mut open).default_size(Vec2::new(840.0,690.0)).show(ctx,|ui|{
            keyboard::block_for_dialog(ctx);
            ui.label("Bring local M3U/M3U8 or Apple Music/iTunes XML playlists into named crates.");
            ui.label("Review local references first. This does not copy audio, unlock protected files or download provider tracks.");
            let available=self.library_crates.pending.is_none() && !self.project.committing() && self.project.dialog_is_closed() && !self.library_closing();
            ui.add_enabled_ui(available,|ui|{
                let mut changed=false;
                for (caption,value) in [("Playlist file",&mut self.library_playlist.path),("Replace path prefix (optional)",&mut self.library_playlist.from),("With local directory (optional)",&mut self.library_playlist.to)] {
                    ui.horizontal(|ui|{let label=ui.label(caption);let response=ui.add(egui::TextEdit::singleline(value).char_limit(4096).desired_width(590.0)).labelled_by(label.id).help(ui,HelpControl::PlaylistImport);response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,response.enabled(),caption));changed|=response.changed();});
                }
                if changed {self.library_playlist.review=None;self.library_playlist.allow_excluded=false;}
                if ui.button("Review playlist file").help(ui,HelpControl::PlaylistImport).clicked() {self.review_playlist();}
            });
            ui.label(&self.library_playlist.message);
            let review=self.library_playlist.review.clone();
            if let Some((revision,review))=review {
                ui.push_id(("playlist-review",revision,self.library_playlist.generation),|ui|{
                    ui.add_enabled_ui(available,|ui|{
                        egui::ScrollArea::vertical().id_salt("playlist-selections").max_height(140.0).show(ui,|ui|{
                            for i in 0..review.playlists.len() {let playlist=&review.playlists[i];let ready=playlist.rows.iter().filter(|&&r|review.rows[r].ready()).count();let mut selected=self.library_playlist.selected.contains(&i);
                                if ui.add_enabled(ready>0,egui::Checkbox::new(&mut selected,format!("{} — {ready}/{} resolved references",playlist.name,playlist.rows.len()))).help(ui,HelpControl::PlaylistImport).changed() {if selected {self.library_playlist.selected.insert(i);}else{self.library_playlist.selected.remove(&i);}}
                                ui.add(egui::Label::new(egui::RichText::new(&playlist.note).small()).truncate());
                            }
                        });
                        let excluded=self.library_playlist.selected.iter().flat_map(|&i|review.playlists[i].rows.iter()).filter(|&&r|!review.rows[r].ready()).count();
                        ui.scope(|ui|{if excluded>0 {ui.checkbox(&mut self.library_playlist.allow_excluded,format!("Import resolved entries and exclude {excluded} reported failures")).help(ui,HelpControl::PlaylistImport);}});
                        if ui.add_enabled(!self.library_playlist.selected.is_empty() && (excluded==0 || self.library_playlist.allow_excluded),egui::Button::new("Import reviewed playlists")).help(ui,HelpControl::PlaylistImport).clicked() {self.import_reviewed_playlists();}
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("playlist-reference-review").max_height(330.0).show_rows(ui,55.0,review.rows.len(),|ui,range|{
                        for i in range {let row=&review.rows[i];ui.allocate_ui(Vec2::new(ui.available_width(),55.0),|ui|{ui.set_min_height(55.0);ui.add(egui::Label::new(format!("{}. {} — {}",i+1,row.title,row.status)).truncate());let location=row.resolved().map(|p|p.display().to_string()).unwrap_or_else(||row.reference.clone());ui.add(egui::Label::new(egui::RichText::new(&location).small()).truncate()).on_hover_text(location);});}
                    });
                });
            }
            if let Some((token,_))=&self.library_crates.pending {
                if ui.button("Cancel playlist operation").help(ui,HelpControl::PlaylistImport).clicked() {self.library_playlist.message=if token.cancel(){"Cancellation requested; waiting for the catalog owner."}else{"Publication began; waiting for its actual receipt."}.into();}
            }
        });self.library_playlist.open&=open;
    }
}
