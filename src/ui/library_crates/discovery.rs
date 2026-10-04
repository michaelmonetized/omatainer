use super::*;
use std::sync::Weak;

#[derive(Default)]
pub(in crate::ui) struct Discovery {
    query: String,
    favorites: bool,
    member: Option<Arc<Selection>>,
    pub(in crate::ui) return_to: Option<library_view::Bookmark>,
    revision: Option<u64>,
    catalog: Weak<crate::library::Catalog>,
    library: Weak<Vec<LibItem>>,
    filtered_query: String,
    filtered_favorites: bool,
    filtered_member: Option<Arc<Selection>>,
    visible: Vec<usize>,
    names: Vec<String>,
    message: String,
    pub(super) published: Option<(u64, Option<CrateId>)>,
    pub(super) generation: u64,
    return_revision: u64,
    #[cfg(test)]
    rebuilds: usize,
}

impl App {
    #[cfg(test)]
    pub(super) fn discovery_evidence(&mut self) -> serde_json::Value {
        self.refresh_crate_discovery();
        serde_json::json!({"ids":self.library_crates.discovery.visible.iter().map(|row|self.library_metadata.catalog.crates.nodes()[self.library_crates.tree[*row].0].id.clone()).collect::<Vec<_>>(),
            "rebuilds":self.library_crates.discovery.rebuilds,"message":self.library_crates.discovery.message,
            "return_available":self.library_crates.discovery.return_to.is_some()})
    }
    /// Rebuild crate-name results only when discovery inputs change.
    /// Takes the exact catalog and library publication; caches at most 4096 tree rows and membership results.
    pub(super) fn refresh_crate_discovery(&mut self) {
        self.refresh_named_crates();
        let state = &mut self.library_crates.discovery;
        let catalog = &self.library_metadata.catalog;
        if state.revision == Some(catalog.crates.revision())
            && state.catalog.as_ptr() == Arc::as_ptr(catalog)
            && state.library.as_ptr() == Arc::as_ptr(&self.library)
            && state.query == state.filtered_query
            && state.favorites == state.filtered_favorites
            && state.member == state.filtered_member
        {
            return;
        }
        state.visible.clear();
        state.names.clear();
        state.message.clear();
        let invalid = state.query.len() > 256 || state.query.chars().any(char::is_control);
        let query = crate::localization::search_key(state.query.trim());
        let membership = state.member.as_ref().map(|selection| {
            let track = catalog.track(&selection.source).filter(|track| {
                track.versions[track.current].fingerprint == selection.fingerprint
            })?;
            self.library_metadata
                .collection_rows()
                .crates_containing(&track.id, &self.library, catalog)
                .map(|nodes| nodes.into_iter().collect::<HashSet<_>>())
        });
        if invalid {
            state.message =
                "Crate search accepts at most 256 UTF-8 bytes without control characters.".into();
        }
        if membership.as_ref().is_some_and(Option::is_none) {
            state.message = "Membership is unavailable for this track version; wait for the current catalog or select it again.".into();
        }
        let by_id: HashMap<_, _> = catalog
            .crates
            .nodes()
            .iter()
            .map(|node| (&node.id, node))
            .collect();
        for (row, &(index, _)) in self.library_crates.tree.iter().enumerate() {
            let node = &catalog.crates.nodes()[index];
            let mut names = vec![node.name.as_str()];
            let mut parent = self
                .library_crates
                .parents
                .get(&node.id)
                .and_then(Option::as_ref);
            while let Some(id) = parent {
                if let Some(node) = by_id.get(id) {
                    names.push(&node.name);
                }
                parent = self.library_crates.parents.get(id).and_then(Option::as_ref);
            }
            names.reverse();
            state.names.push(names.join(" / "));
            if !invalid
                && (!state.favorites || node.favorite)
                && (query.is_empty()
                    || crate::localization::search_key(&node.name).contains(&query))
                && membership.as_ref().is_none_or(|members| {
                    members
                        .as_ref()
                        .is_some_and(|members| members.contains(&index))
                })
            {
                state.visible.push(row);
            }
        }
        state.revision = Some(catalog.crates.revision());
        state.catalog = Arc::downgrade(catalog);
        state.library = Arc::downgrade(&self.library);
        state.filtered_query.clone_from(&state.query);
        state.filtered_favorites = state.favorites;
        state.filtered_member = state.member.clone();
        state.generation = state.generation.wrapping_add(1);
        #[cfg(test)]
        {
            state.rebuilds += 1;
        }
    }

    /// Enter a discovered crate while retaining the original track view.
    /// Takes its stable identity; preserves one return point across multiple result selections.
    pub(super) fn choose_discovered_crate(&mut self, id: CrateId) {
        if self.library_metadata.catalog.crates.node(&id).is_none() {
            return;
        }
        let retained = self.library_crates.discovery.return_to.take();
        if retained.is_none() {
            self.library_crates.discovery.return_revision = self
                .library_crates
                .discovery
                .return_revision
                .wrapping_add(1);
        }
        let previous = retained.unwrap_or_else(|| self.library_bookmark());
        self.choose_named_crate(Some(id));
        self.lib_filter.clear();
        self.refresh_library_view();
        self.library_crates.discovery.return_to = Some(previous);
        self.publish_library_selection();
    }

    /// Publish the bounded filtered crate list to the MIDI dispatch bridge.
    /// Takes current discovery state; updates the list only after a real filter or manual-selection change.
    pub(in crate::ui) fn publish_crate_navigation(&mut self) {
        self.refresh_crate_discovery();
        let state = &self.library_crates.discovery;
        self.engine
            .ui_requests
            .publish_crate_return(state.return_to.as_ref().map(|_| state.return_revision));
        let signature = (state.generation, self.library_crates.selected.clone());
        if state.published.as_ref() == Some(&signature) {
            return;
        }
        let ids: Vec<_> = state
            .visible
            .iter()
            .map(|row| {
                self.library_metadata.catalog.crates.nodes()[self.library_crates.tree[*row].0]
                    .id
                    .clone()
            })
            .collect();
        let cursor = self
            .library_crates
            .selected
            .as_ref()
            .and_then(|id| ids.iter().position(|candidate| candidate == id))
            .unwrap_or(usize::MAX);
        self.engine.ui_requests.publish_crates(
            Arc::new(PublishedCrates {
                catalog: Arc::downgrade(&self.library_metadata.catalog),
                ids,
            }),
            cursor,
        );
        self.library_crates.discovery.published = Some(signature);
    }

    /// Apply a captured controller row without borrowing a later result's identity.
    /// Takes its list epoch and stable crate ID; ignores stale or removed results.
    pub(in crate::ui) fn handle_crate_browse(
        &mut self,
        request: crate::engine::ui_requests::CrateBrowseRequest,
    ) {
        if request.epoch != self.engine.ui_requests.crate_epoch() {
            return;
        }
        self.refresh_crate_discovery();
        let id = CrateId(request.id);
        if !self
            .library_crates
            .discovery
            .visible
            .get(request.index)
            .is_some_and(|row| {
                self.library_metadata.catalog.crates.nodes()[self.library_crates.tree[*row].0].id
                    == id
            })
        {
            return;
        }
        self.library_crates.open = true;
        // Mark the acknowledgement before entering the crate: entering publishes
        // track navigation, while later queued crate steps retain their cursor.
        self.library_crates.discovery.published =
            Some((self.library_crates.discovery.generation, Some(id.clone())));
        self.choose_discovered_crate(id);
    }

    /// Consume only the return point captured at controller admission.
    /// Takes its token; ignores requests for an already returned or replaced bookmark.
    pub(in crate::ui) fn handle_crate_return(&mut self, token: u64) {
        if self.library_crates.discovery.return_revision == token {
            self.return_from_crate_discovery();
        }
    }

    /// Restore the track view captured before choosing a discovery result.
    /// Takes current discovery state; returns whether a bookmark was available.
    pub(super) fn return_from_crate_discovery(&mut self) -> bool {
        let Some(bookmark) = self.library_crates.discovery.return_to.take() else {
            return false;
        };
        self.restore_library_bookmark(bookmark);
        self.library_crates.discovery.return_revision = self
            .library_crates
            .discovery
            .return_revision
            .wrapping_add(1);
        self.library_crates.discovery.published = None;
        true
    }

    /// Capture a selected track before revealing its direct memberships.
    /// Takes current track selection; opens exact-version results without changing the track query.
    fn find_selected_crates(&mut self) {
        let Some(item) = self.selected_library_item() else {
            self.library_crates.message =
                "Select a saved library track to reveal its crates.".into();
            return;
        };
        let selection = Arc::new(Selection {
            source: item.source.clone(),
            title: item.title.clone(),
            fingerprint: item.fingerprint,
        });
        let state = &mut self.library_crates.discovery;
        state.member = Some(selection);
        state.query.clear();
        state.favorites = false;
        self.refresh_crate_discovery();
    }

    /// Render bounded crate discovery and its keyboard-accessible results.
    /// Takes the manager UI; leaves track search and catalog persistence to their existing owners.
    pub(super) fn crate_discovery_ui(&mut self, ui: &mut egui::Ui) {
        self.refresh_crate_discovery();
        let keyboard = ui.input_mut(|input| {
            [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End]
                .into_iter()
                .find(|key| input.consume_key(egui::Modifiers::ALT, *key))
        });
        if let Some(key) = keyboard {
            let state = &self.library_crates.discovery;
            let current = self.library_crates.selected.as_ref().and_then(|id| {
                state.visible.iter().position(|row| {
                    &self.library_metadata.catalog.crates.nodes()[self.library_crates.tree[*row].0]
                        .id
                        == id
                })
            });
            let row = match key {
                Key::ArrowUp => current.unwrap_or(state.visible.len()).saturating_sub(1),
                Key::ArrowDown => current
                    .map_or(0, |row| row.saturating_add(1))
                    .min(state.visible.len().saturating_sub(1)),
                Key::End => state.visible.len().saturating_sub(1),
                _ => 0,
            };
            if let Some(row) = state.visible.get(row) {
                let id = self.library_metadata.catalog.crates.nodes()
                    [self.library_crates.tree[*row].0]
                    .id
                    .clone();
                self.choose_discovered_crate(id);
            }
        }
        ui.push_id("crate-discovery", |ui| {
            let state = &mut self.library_crates.discovery;
            ui.horizontal(|ui| {
                let label = ui.label(tr!("Search crate names"));
                ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .char_limit(256)
                        .desired_width(190.0),
                )
                .labelled_by(label.id)
                .help(ui, HelpControl::NamedCrates)
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        true,
                        "Search crate names",
                    )
                });
                ui.checkbox(&mut state.favorites, tr!("Favorite crates only"))
                    .help(ui, HelpControl::NamedCrates);
            });
            if ui
                .button(tr!("Find crates containing selected track"))
                .help(ui, HelpControl::NamedCrates)
                .clicked()
            {
                self.find_selected_crates();
            }
            ui.push_id(self.library_crates.discovery.return_revision, |ui| {
                if ui
                    .add_enabled(
                        self.library_crates.discovery.return_to.is_some(),
                        egui::Button::new(tr!("Return to previous crate view")),
                    )
                    .help(ui, HelpControl::NamedCrates)
                    .clicked()
                {
                    self.return_from_crate_discovery();
                }
            });
            if let Some(member) = &self.library_crates.discovery.member {
                ui.label(crate::localization::format(
                    "Crates containing {track}",
                    &[member.title.clone()],
                ));
                if ui
                    .button(tr!("Clear membership filter"))
                    .help(ui, HelpControl::NamedCrates)
                    .clicked()
                {
                    self.library_crates.discovery.member = None;
                }
            }
            self.refresh_crate_discovery();
            ui.label(&self.library_crates.discovery.message);
            ui.label(tr!(
                "Alt+Up/Down browses crate results; Alt+Home/End selects the first/last."
            ));
        });
        let discovered = !self.library_crates.discovery.query.trim().is_empty()
            || self.library_crates.discovery.favorites
            || self.library_crates.discovery.member.is_some();
        ui.push_id(
            ("crate-tree", self.library_crates.discovery.generation),
            |ui| {
                let count = self.library_crates.discovery.visible.len();
                let current = self
                    .library_crates
                    .selected
                    .as_ref()
                    .and_then(|id| {
                        self.library_crates
                            .discovery
                            .visible
                            .iter()
                            .position(|row| {
                                let (index, _) = self.library_crates.tree[*row];
                                &self.library_metadata.catalog.crates.nodes()[index].id == id
                            })
                    })
                    .map(|row| row + 1)
                    .unwrap_or(0);
                let mut number = current as f32;
                preferences::float_control(
                    ui,
                    "Crate tree row (0 = All tracks)",
                    &mut number,
                    0.0,
                    count as f32,
                    1.0,
                    " row",
                    HelpControl::NamedCrates,
                );
                let mut chosen = None;
                if number.round() as usize != current {
                    chosen = Some(
                        (number.round() as usize)
                            .checked_sub(1)
                            .and_then(|index| self.library_crates.discovery.visible.get(index))
                            .map(|row| {
                                self.library_metadata.catalog.crates.nodes()
                                    [self.library_crates.tree[*row].0]
                                    .id
                                    .clone()
                            }),
                    );
                }
                if ui
                    .selectable_label(self.library_crates.selected.is_none(), tr!("All tracks"))
                    .help(ui, HelpControl::NamedCrates)
                    .clicked()
                {
                    chosen = Some(None);
                }
                let output = egui::ScrollArea::vertical()
                    .id_salt("named-crate-tree")
                    .max_height(150.0)
                    .show_rows(ui, 20.0, count, |ui, range| {
                        for row in range {
                            let tree_row = self.library_crates.discovery.visible[row];
                            let (index, depth) = self.library_crates.tree[tree_row];
                            let node = &self.library_metadata.catalog.crates.nodes()[index];
                            ui.push_id((&node.id, "tree-row"), |ui| {
                                ui.horizontal(|ui| {
                                    if !discovered {
                                        ui.add_space(depth as f32 * 12.0);
                                    }
                                    let name = if discovered {
                                        &self.library_crates.discovery.names[tree_row]
                                    } else {
                                        &node.name
                                    };
                                    let name = if node.favorite {
                                        format!("★ {name}")
                                    } else {
                                        name.clone()
                                    };
                                    let label = if node.smart_rule.is_some() {
                                        let count = self
                                            .library_metadata
                                            .collection_rows()
                                            .smart_rows(
                                                &node.id,
                                                &self.library,
                                                &self.library_metadata.catalog,
                                            )
                                            .map(|rows| rows.len().to_string())
                                            .unwrap_or_else(|| "…".into());
                                        crate::localization::format(
                                            "{name} · {count} automatic tracks",
                                            &[name, count],
                                        )
                                    } else if node.annotation_rule.is_some() {
                                        crate::localization::format(
                                            "{name} · annotation rule",
                                            &[name],
                                        )
                                    } else {
                                        crate::localization::format(
                                            "{} · {} tracks",
                                            &[name, node.members.len().to_string()],
                                        )
                                    };
                                    if ui
                                        .selectable_label(
                                            self.library_crates.selected.as_ref() == Some(&node.id),
                                            label,
                                        )
                                        .help(ui, HelpControl::NamedCrates)
                                        .clicked()
                                    {
                                        chosen = Some(Some(node.id.clone()));
                                    }
                                });
                            });
                        }
                    });
                accessibility::scrollbars(ui, "Named crate tree", &output);
                if let Some(id) = chosen {
                    match id {
                        Some(id) if discovered => self.choose_discovered_crate(id),
                        id => {
                            self.choose_named_crate(id);
                            self.library_crates.discovery.published = None;
                        }
                    }
                }
            },
        );
    }
}

struct PublishedCrates {
    catalog: Weak<crate::library::Catalog>,
    ids: Vec<CrateId>,
}
impl crate::engine::ui_requests::CrateView for PublishedCrates {
    fn len(&self) -> Option<usize> {
        self.catalog.upgrade()?;
        Some(self.ids.len())
    }
    fn id(&self, index: usize) -> Option<String> {
        self.catalog.upgrade()?;
        Some(self.ids.get(index)?.0.clone())
    }
}
