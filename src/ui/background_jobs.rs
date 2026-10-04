//! Worker progress is separate from renderer application and durable publication.
use super::*;
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
}
impl App {
    /// Inspect and cancel captured background requests.
    /// Takes the native context; reports actual work/reservations and cancels only the chosen stable request identity.
    pub(super) fn background_jobs_ui(&mut self, ctx: &egui::Context) {
        if !self.background_jobs.open {
            return;
        }
        let scheduler = self.engine.cmd.performance().jobs().clone();
        let snapshot = scheduler.snapshot();
        let mut open = true;
        egui::Window::new(tr!("Background jobs")).id(egui::Id::new("background-jobs")).open(&mut open).default_width(600.0).vscroll(true).show(ctx,|ui|{
            ui.label(format!("{} running · {} MiB reserved of {} MiB",snapshot.running,snapshot.reserved/crate::background::MIB,crate::background::MEMORY_BYTES/crate::background::MIB));
            ui.label("At most two optional workers and one audio decode run together. A deck load gets the next turn; performance protection cancels optional work.");
            for row in snapshot.rows.iter().rev(){ui.push_id(row.id,|ui|{
                ui.horizontal_wrapped(|ui|{
                    ui.strong(row.kind.title());ui.label(row.phase.title());ui.small(format!("{} MiB",row.bytes/crate::background::MIB));
                    if row.nice.is_some(){ui.small("Worker priorities applied");}
                    let response=ui.add_enabled(row.cancellable,egui::Button::new(tr!("Cancel job")));
                    accessibility::button(ui,&response,&format!("Cancel {} job {}",row.kind.title(),row.id),None);
                    help::annotate(ui,&response,HelpControl::BackgroundCancel);
                    if response.clicked(){self.status=if scheduler.cancel(row.id){"Background cancellation requested; its workflow reports the final result".into()}else{"Worker finished before cancellation; inspect its workflow result".into()};}
                });
                if let Some(total)=row.total.filter(|total|*total>0){ui.add(egui::ProgressBar::new((row.done.min(total) as f64/total as f64) as f32).show_percentage());}
                else if row.cancellable {ui.horizontal(|ui|{ui.spinner();ui.label(if row.done>0 {format!("{} work units processed; total unknown",row.done)}else{"Waiting for measured progress".into()});});}
            });}
            if snapshot.rows.is_empty(){ui.label("No background requests in this session");}
            ui.separator();
            ui.label("A finished worker may still be waiting for your choice or audio acknowledgment. See its original window for the final result. Cancellation does not undo an already completed write.");
        });
        self.background_jobs.open = open;
        if snapshot.rows.iter().any(|row| row.cancellable) {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Action, ActionRequest};
    use std::sync::{mpsc, Arc, Mutex};
    fn frame(
        fixture: &mut test_support::Fixture,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        fixture.rt.process(&mut [0.0; 256]);
        fixture.rt.publish_for_test();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1400.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| fixture.app.update_frame(ctx),
        )
    }
    #[test]
    fn native_job_monitor_cancels_the_actual_indexer_and_preserves_loaded_media_and_the_prior_crate(
    ) {
        let files = crate::engine::media_analysis::tests::Files::new();
        files.source(
            "one.wav",
            &crate::engine::media_analysis::tests::wav(8000, 8000, 1, false),
        );
        let mut fixture = test_support::Fixture::new(256);
        fixture.rt.apply(Command::DeckPlay { deck: 0 });
        let original = fixture.rt.decks[0].audio.clone().unwrap();
        let baseline = fixture.app.library.clone();
        let (entered, seen) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let wait = Arc::new(Mutex::new(wait));
        let once = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let options = library_scan::Options {
            before_entry: Some(Arc::new(move |_| {
                if !once.swap(true, std::sync::atomic::Ordering::AcqRel) {
                    entered.send(()).unwrap();
                    wait.lock().unwrap().recv().unwrap();
                }
            })),
        };
        assert!(fixture.app.library_scan.start_with(
            vec![files.0.clone()],
            baseline.clone(),
            options
        ));
        seen.recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        fixture.app.background_jobs.open = true;
        let output = frame(&mut fixture, &ctx, 1.0, vec![]);
        let target = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .find_map(|(id, node)| {
                (node
                    .label()
                    .is_some_and(|label| label.starts_with("Cancel Library indexing job"))
                    && node.supports_action(Action::Click))
                .then_some(id)
            })
            .unwrap();
        frame(
            &mut fixture,
            &ctx,
            1.1,
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target,
                action: Action::Click,
                data: None,
            })],
        );
        assert!(fixture.app.status.contains("cancellation requested"));
        resume.send(()).unwrap();
        let until = Instant::now() + std::time::Duration::from_secs(3);
        let mut time = 1.2;
        while fixture.app.library_scan.active() {
            frame(&mut fixture, &ctx, time, vec![]);
            time += 0.02;
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(Arc::ptr_eq(&fixture.app.library, &baseline));
        assert!(Arc::ptr_eq(
            fixture.rt.decks[0].audio.as_ref().unwrap(),
            &original
        ));
        assert!(fixture.rt.decks[0].playing);
        assert_eq!(
            fixture
                .app
                .engine
                .cmd
                .performance()
                .jobs()
                .snapshot()
                .reserved,
            0
        );
    }
}
