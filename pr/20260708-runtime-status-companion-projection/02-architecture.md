# Architecture

`RuntimeLifecycleService` owns observation sequencing for runtime status. It
captures projection/process/product presence facts and now also captures desktop
companion status DTOs.

`RuntimeStatusReport` owns classification and rendering only. It stores the
already captured desktop companion DTO list and serializes it without reaching
back into plugin state.
