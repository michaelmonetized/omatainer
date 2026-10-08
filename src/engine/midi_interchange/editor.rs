use super::*;
impl Request {
    /// Prepare an atomic edit across captured MIDI clips.
    /// Takes a coherent project, validated drafts and cancellation; returns one guarded request, acknowledgement and next documents.
    pub(crate) fn prepare_edits(
        mut captured: project::Captured,
        edits: Vec<super::super::midi_edit::Request>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(Self, Ack, Vec<Arc<Document>>), String> {
        let mut cancelled = || cancel.load(std::sync::atomic::Ordering::Acquire);
        checkpoint(&mut cancelled)?;
        if edits.is_empty() || edits.len() > 64 {
            return Err("Choose 1–64 MIDI clips for a combined edit".into());
        }
        let metadata_baseline = Some(captured.checkpoint);
        let epoch = captured.checkpoint.epoch;
        let namespace = captured
            .state
            .session
            .as_ref()
            .ok_or("Session identity is unavailable")?
            .namespace;
        let mut seen = BTreeSet::new();
        let mut targets = Vec::with_capacity(edits.len());
        let mut next = Vec::with_capacity(edits.len());
        for edit in edits {
            checkpoint(&mut cancelled)?;
            let base = &edit.baseline;
            if !seen.insert((base.track, base.scene)) {
                return Err("The same clip appears twice in the combined edit".into());
            }
            let layout = captured.state.session.as_ref().unwrap();
            if base.song_context != captured.state.musical_context || base.epoch != epoch
                || base
                    .track_identity
                    .is_none_or(|r| !layout.resolves(session::Axis::Track, base.track as usize, r))
                || base
                    .scene_identity
                    .is_none_or(|r| !layout.resolves(session::Axis::Scene, base.scene as usize, r))
            {
                return Err(
                    "A captured MIDI clip identity changed; all drafts are retained".into(),
                );
            }
            let old = captured
                .state
                .tracks
                .get_mut(base.track as usize)
                .and_then(|t| t.clips.get_mut(base.scene as usize))
                .ok_or("A captured MIDI clip no longer exists")?;
            if old.kind != base.kind
                || old.kind == ClipKind::Audio
                || old.audio.is_some()
                || old.name != base.name
                || old.bars != base.bars
                || old.region != base.region
                || old.notes != base.notes
                || old.lanes != base.lanes
                || old.properties.context != base.context
            {
                return Err("A captured MIDI clip changed; no combined edit was prepared".into());
            }
            let (_, _, document) = super::super::midi_edit::Request::with_context(
                base.clone(),
                edit.name.clone(),
                edit.region,
                edit.notes.clone(),
                edit.lanes.clone(),
                edit.context,
            )?;
            next.push(document);
            if edit.unchanged() {
                continue;
            }
            let reserved_lane_bytes = old.lanes.as_ref().map_or(0, |l| l.bytes())
                + edit.lanes.as_ref().map_or(0, |l| l.bytes());
            let replacement = Clip {
                properties: super::super::clip_management::Properties { context: edit.context, ..old.properties },
                audio_region: None,
                lanes: edit.lanes.clone(),
                region: Some(edit.region),
                kind: ClipKind::Midi,
                name: edit.name.clone(),
                bars: (edit.region.end / 4.0) as f32,
                notes: edit.notes.clone(),
                gain: old.gain,
                audio: None,
            };
            old.name = edit.name;
            old.kind = ClipKind::Midi;
            old.region = Some(edit.region);
            old.bars = (edit.region.end / 4.0) as f32;
            old.notes = edit.notes;
            old.lanes = edit.lanes;
            old.properties.context = edit.context;
            targets.push(Target {
                baseline: edit.baseline,
                replacement,
                spare_notes: Vec::with_capacity(project::MAX_NOTES_PER_CLIP),
                reserved_lane_bytes,
            });
        }
        if targets.is_empty() {
            return Err("The selected MIDI clips contain no changes".into());
        }
        captured
            .state
            .validate(&captured.media)
            .map_err(|e| e.to_string())?;
        struct Size<'a> {
            bytes: usize,
            limit: usize,
            cancel: &'a std::sync::atomic::AtomicBool,
        }
        impl std::io::Write for Size<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(std::io::Error::other("Combined MIDI edit cancelled"));
                }
                self.bytes = self.bytes.saturating_add(bytes.len());
                if self.bytes > self.limit {
                    return Err(std::io::Error::other(
                        "Combined MIDI edit exceeds native project metadata limits",
                    ));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let limit = captured
            .state
            .import_metadata_limits()
            .max_metadata_bytes
            .min(crate::project_file::DEFAULT_METADATA_LIMIT - 128 * 1024);
        serde_json::to_writer(
            Size {
                bytes: 0,
                limit,
                cancel,
            },
            &captured.state,
        )
        .map_err(|e| e.to_string())?;
        checkpoint(&mut cancelled)?;
        let ack = Ack::new();
        Ok((
            Self {
                targets,
                epoch,
                metadata_baseline,
                session_namespace: Some(namespace),
                baseline_bpm: captured.state.bpm,
                baseline_scene_timing: captured.state.scene_timing,
                baseline_conductor: captured.state.conductor.clone(),
                conductor: captured.state.conductor.clone(),
                change_conductor: false,
                ack: ack.clone(),
            },
            ack,
            next,
        ))
    }
}
