# Intentional video cancellation status

Base: PR #485. Cancelled import/render workers replace their running message with
an explicit cancellation result after the worker finishes. Preview cancellation
keeps its existing viewer status, and successful committed renders retain their
actual published path or durability warning.

Six native video/model/decoder tests pass, including the real GUI render-cancel
path and its terminal status. Eight license/package fixtures and git diff --check
pass. Receipts: adjacent video-cancel-focused.log, video-cancel-license-tests.log
and video-cancel-validation.json. The Python launcher workaround from #122 is
retained for the package fixtures. This small follow-up was checked in the debug
native executable; a separate full release workload run is not claimed.
