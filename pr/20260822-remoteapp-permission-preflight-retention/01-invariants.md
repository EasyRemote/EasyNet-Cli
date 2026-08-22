# Invariants

1. Product-flow coverage must prove denied preflight stays in the picker.
2. Frontend boundary checks must reject `rdCheckPermission` writing
   `error: permissionResult.message`.
3. Product audit must keep the frontend lifecycle row partial until real
   Browser/Tauri E2E exists.
