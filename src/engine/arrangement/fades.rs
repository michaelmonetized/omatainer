use super::*;

fn audio_length(model: &Model, instance: Instance, media: &[Arc<Sample>]) -> Result<f64, String> {
    let source = model
        .sources
        .iter()
        .find(|s| s.id == instance.source)
        .ok_or("Missing fade source")?;
    let clip = &source.clip;
    if clip.kind != ClipKind::Audio {
        return Err("Fades require audio placements".into());
    }
    let audio = clip
        .audio
        .and_then(|i| media.get(i))
        .ok_or("Missing fade audio")?;
    if let Some(clock) = &source.audio_clock {
        let conductor = clock.conductor.prepare()?;
        let region = clip
            .audio_region
            .ok_or("Aligned fade source has no audio region")?;
        let seconds = (region.end - region.start) as f64 / f64::from(audio.sr);
        return Ok(
            conductor.beat_at_seconds(conductor.seconds_at(clock.origin) + seconds) - clock.origin,
        );
    }
    clip.audio_region
        .map_or(Ok(f64::from(clip.bars) * 4.0), |r| {
            r.prepare(audio)
                .map(|p| p.duration_beats)
                .map_err(str::to_owned)
        })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

impl Model {
    fn placement(&self, id: u64) -> Result<Instance, String> {
        self.instances
            .iter()
            .find(|i| i.id == id)
            .copied()
            .ok_or_else(|| "Audio placement was deleted".into())
    }
    fn members(&self, instance: Instance) -> Vec<u64> {
        self.instances
            .iter()
            .filter(|i| {
                i.id == instance.id || instance.fade_link != 0 && i.fade_link == instance.fade_link
            })
            .map(|i| i.id)
            .collect()
    }
    fn fade_geometry(
        &self,
        instances: &[Instance],
        next_id: u64,
        media: &[Arc<Sample>],
    ) -> Result<(), String> {
        let mut checked_links = std::collections::HashSet::new();
        let by_id: std::collections::HashMap<_, _> = instances.iter().map(|i| (i.id, i)).collect();
        let mut incoming_counts = std::collections::HashMap::<u64, usize>::new();
        for instance in instances {
            if let Some(id) = instance.crossfade {
                *incoming_counts.entry(id).or_default() += 1;
            }
        }
        for instance in instances {
            if instance.fades.is_some() || instance.fade_link != 0 || instance.crossfade.is_some() {
                let length = audio_length(self, *instance, media)?;
                if !instance.repeating && instance.offset + instance.duration > length + 1e-9
                    || instance.fades.is_some_and(|f| !f.valid(instance.duration))
                {
                    return Err("Audio fade exceeds placement or source bounds".into());
                }
            }
            if instance.fade_link != 0 && checked_links.insert(instance.fade_link) {
                if instance.fade_link >= next_id
                    || self.sources.iter().any(|s| s.id == instance.fade_link)
                    || by_id.contains_key(&instance.fade_link)
                {
                    return Err("Invalid fade link identity".into());
                }
                let linked: Vec<_> = instances
                    .iter()
                    .filter(|i| i.fade_link == instance.fade_link)
                    .collect();
                if linked.len() < 2
                    || linked.iter().any(|i| {
                        !close(i.start, instance.start)
                            || !close(i.duration, instance.duration)
                            || i.fades != instance.fades
                    })
                    || linked
                        .iter()
                        .map(|i| i.track)
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != linked.len()
                {
                    return Err("Linked fades require aligned audio on distinct tracks".into());
                }
            }
            if let Some(id) = instance.crossfade {
                let incoming = **by_id.get(&id).ok_or("Crossfade partner was deleted")?;
                let overlap = instance.start + instance.duration - incoming.start;
                let outgoing_fade = instance.fades.ok_or("Crossfade has no outgoing envelope")?;
                let incoming_fade = incoming.fades.ok_or("Crossfade has no incoming envelope")?;
                if instance.id == incoming.id
                    || instance.track != incoming.track
                    || instance.repeating
                    || incoming.repeating
                    || incoming.start <= instance.start
                    || incoming.start + incoming.duration <= instance.start + instance.duration
                    || overlap <= 0.0
                    || !close(overlap, outgoing_fade.fade_out)
                    || !close(overlap, incoming_fade.fade_in)
                    || outgoing_fade.out_curve != incoming_fade.in_curve
                    || incoming_counts.get(&id) != Some(&1)
                {
                    return Err(
                        "Crossfade edges, curves or identities changed; edit or unlink the pair"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }
    /// Validate retained fade groups and complementary partner edges.
    /// Takes immutable source media; returns whether linked timing, independent handles and curve relationships remain saveable.
    pub(super) fn validate_fades(&self, media: &[Arc<Sample>]) -> Result<(), String> {
        self.fade_geometry(&self.instances, self.next_id, media)
    }
    /// Edit a placement and its aligned fade links atomically.
    /// Takes stable identity, reviewed replacement and shared media; returns changed metadata or refuses missing source handles and broken crossfade geometry.
    pub(crate) fn edit_fades(
        &mut self,
        id: u64,
        replacement: Instance,
        media: &[Arc<Sample>],
    ) -> Result<(), String> {
        let original = self.placement(id)?;
        if replacement.id != id
            || replacement.source != original.source
            || replacement.fade_link != original.fade_link
            || replacement.crossfade != original.crossfade
        {
            return Err("Placement identity changed during fade editing".into());
        }
        let mut edited = self.instances.clone();
        for member in self.members(original) {
            let target = edited.iter_mut().find(|i| i.id == member).unwrap();
            if member == id {
                *target = replacement;
            } else {
                target.start += replacement.start - original.start;
                target.offset += replacement.offset - original.offset;
                target.duration = replacement.duration;
                target.fades = replacement.fades;
            }
        }
        self.fade_geometry(&edited, self.next_id, media)?;
        for target in &edited {
            if ![target.start, target.offset, target.duration]
                .into_iter()
                .all(|n| n.is_finite() && n >= 0.0)
                || target.duration < 0.000001
                || target.start + target.duration > MAX_BEATS
            {
                return Err("Linked fade edit exceeds timeline bounds".into());
            }
        }
        self.instances = edited;
        Ok(())
    }
    /// Link aligned audio envelopes across retained track identities.
    /// Takes two placements and immutable media; returns one shared fade identity while preserving each source offset, or refuses incompatible timing.
    pub(crate) fn link_fades(
        &mut self,
        first: u64,
        second: u64,
        media: &[Arc<Sample>],
    ) -> Result<(), String> {
        let a = self.placement(first)?;
        let b = self.placement(second)?;
        audio_length(self, a, media)?;
        audio_length(self, b, media)?;
        if a.id == b.id || !close(a.start, b.start) || !close(a.duration, b.duration) {
            return Err("Choose aligned audio placements on different tracks".into());
        }
        let mut edited = self.instances.clone();
        let members: std::collections::HashSet<_> =
            self.members(a).into_iter().chain(self.members(b)).collect();
        let link = if a.fade_link != 0 {
            a.fade_link
        } else if b.fade_link != 0 {
            b.fade_link
        } else {
            self.next_id
        };
        let next_id = if link == self.next_id {
            self.next_id
                .checked_add(1)
                .ok_or("Fade identities exhausted")?
        } else {
            self.next_id
        };
        for i in &mut edited {
            if members.contains(&i.id) {
                i.fade_link = link;
                i.fades = a.fades;
            }
        }
        self.fade_geometry(&edited, next_id, media)?;
        self.instances = edited;
        self.next_id = next_id;
        Ok(())
    }
    /// Release one placement from its fade link.
    /// Takes a stable placement identity; retains all audible envelopes and removes a remaining single-member link.
    pub(crate) fn unlink_fades(&mut self, id: u64) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            let link = instance.fade_link;
            instance.fade_link = 0;
            self.prune_link(link);
        }
    }
    fn prune_link(&mut self, link: u64) {
        if link != 0
            && self
                .instances
                .iter()
                .filter(|i| i.fade_link == link)
                .count()
                < 2
        {
            for i in &mut self.instances {
                if i.fade_link == link {
                    i.fade_link = 0;
                }
            }
        }
    }
    /// Build or resize complementary adjacent and overlapping crossfades.
    /// Takes outgoing/incoming identities, overlap duration, curve and source media; changes all aligned linked tracks together or refuses the entire edit when handles are missing.
    pub(crate) fn crossfade(
        &mut self,
        first: u64,
        second: u64,
        length: f64,
        curve: f32,
        media: &[Arc<Sample>],
    ) -> Result<(), String> {
        if !length.is_finite()
            || !(0.000001..=MAX_BEATS).contains(&length)
            || !curve.is_finite()
            || !(-1.0..=1.0).contains(&curve)
        {
            return Err(
                "Crossfade needs a finite positive length and curve between -1 and 1".into(),
            );
        }
        let a = self.placement(first)?;
        let b = self.placement(second)?;
        if a.start >= b.start
            || a.start + a.duration < b.start - 1e-9
            || b.start + b.duration <= a.start + a.duration
        {
            return Err(
                "Choose adjacent or overlapping audio with the outgoing placement first".into(),
            );
        }
        let outgoing = self.members(a);
        let incoming = self.members(b);
        if outgoing.len() != incoming.len() {
            return Err("Crossfade links need matching retained tracks on both sides".into());
        }
        let center = (a.start + a.duration + b.start) * 0.5;
        let mut edited = self.instances.clone();
        for id in outgoing {
            let old = self.placement(id)?;
            let next = incoming
                .iter()
                .filter_map(|id| self.placement(*id).ok())
                .find(|i| i.track == old.track)
                .ok_or("Crossfade links have different tracks")?;
            if old.repeating || next.repeating {
                return Err("Crossfades require one-shot audio placements".into());
            }
            let mut out = old;
            let mut inc = next;
            out.duration = center + length * 0.5 - out.start;
            inc.start = center - length * 0.5;
            inc.offset += inc.start - next.start;
            inc.duration = next.start + next.duration - inc.start;
            if inc.offset < 0.0
                || inc.start < 0.0
                || out.duration <= 0.0
                || inc.duration <= 0.0
                || out.offset + out.duration > audio_length(self, out, media)? + 1e-9
                || inc.offset + inc.duration > audio_length(self, inc, media)? + 1e-9
            {
                return Err("Crossfade lacks source handles; the complete edit was refused".into());
            }
            if old.crossfade.is_some_and(|id| id != next.id)
                || self
                    .instances
                    .iter()
                    .any(|i| i.crossfade == Some(next.id) && i.id != old.id)
            {
                return Err("Unlink the existing crossfade before changing partners".into());
            }
            let mut out_fades = out.fades.unwrap_or_default();
            let mut in_fades = inc.fades.unwrap_or_default();
            out_fades.fade_out = length;
            out_fades.out_curve = curve;
            in_fades.fade_in = length;
            in_fades.in_curve = curve;
            out.fades = Some(out_fades);
            inc.fades = Some(in_fades);
            out.crossfade = Some(inc.id);
            *edited.iter_mut().find(|i| i.id == out.id).unwrap() = out;
            *edited.iter_mut().find(|i| i.id == inc.id).unwrap() = inc;
        }
        self.fade_geometry(&edited, self.next_id, media)?;
        self.instances = edited;
        Ok(())
    }
    /// Keep envelopes while releasing a crossfade relationship.
    /// Takes either pair member; removes the retained partner link without changing source coordinates or gain.
    pub(crate) fn unlink_crossfade(&mut self, id: u64) {
        let members = self
            .placement(id)
            .map(|i| self.members(i))
            .unwrap_or_else(|_| vec![id]);
        for i in &mut self.instances {
            if members.contains(&i.id)
                || i.crossfade
                    .is_some_and(|partner| members.contains(&partner))
            {
                i.crossfade = None;
            }
        }
    }
    /// Delete a stable placement and retire its fade relationships.
    /// Takes an instance identity; removes it and retains remaining envelopes with no dangling pair or single-member fade link.
    pub(crate) fn remove_instance(&mut self, id: u64) {
        let link = self
            .instances
            .iter()
            .find(|i| i.id == id)
            .map_or(0, |i| i.fade_link);
        self.instances.retain(|i| i.id != id);
        self.unlink_crossfade(id);
        self.prune_link(link);
    }
}

#[cfg(test)]
mod tests;
