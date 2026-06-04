use crate::core;
use crate::gui::app::AppState;
use crate::hub::{self, HubInfo};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

struct JobTargets {
    selected: Vec<core::UnityInstall>,
    all_paths: Vec<PathBuf>,
    all_exe_paths: Vec<PathBuf>,
    hubs: Vec<HubInfo>,
    hub_selected: Vec<bool>,
    is_unpatch: bool,
}

pub fn start_selected(app: &mut AppState) -> bool {
    let Some(targets) = collect_targets(app) else {
        return false;
    };

    let (tx, rx) = mpsc::channel();
    app.running = true;
    app.done = false;
    app.success_count = 0;
    app.error_count = 0;
    app.first_error.clear();
    app.cleaned_globals = false;
    app.last_op_unpatch = targets.is_unpatch;
    app.rx = Some(rx);

    if targets.is_unpatch {
        thread::spawn(move || run_unpatch(tx, targets));
    } else {
        thread::spawn(move || run_patch(tx, targets));
    }

    true
}

fn collect_targets(app: &AppState) -> Option<JobTargets> {
    if app.running {
        return None;
    }

    let mode = app.selection_mode?;
    let selected = app
        .installs
        .iter()
        .enumerate()
        .filter(|(idx, _)| *app.selected.get(*idx).unwrap_or(&false))
        .map(|(_, inst)| inst.clone())
        .collect::<Vec<_>>();

    let any_hub_selected = app.hub_selected.iter().any(|s| *s);

    if selected.is_empty() && !any_hub_selected {
        return None;
    }

    Some(JobTargets {
        selected,
        all_paths: app.installs.iter().map(|i| i.dll_path.clone()).collect(),
        all_exe_paths: app.installs.iter().map(|i| i.exe_path.clone()).collect(),
        hubs: app.hubs.clone(),
        hub_selected: app.hub_selected.clone(),
        is_unpatch: mode,
    })
}

fn run_patch(tx: mpsc::Sender<String>, targets: JobTargets) {
    let mut ok = false;

    for inst in &targets.selected {
        if core::is_patched(&inst.dll_path) {
            tx.send(format!("TOK  {}  already patched, skipped", inst.version))
                .ok();
            ok = true;
            continue;
        }
        tx.send(format!("..  {}  patching DLL...", inst.version))
            .ok();
        match core::patch_dll(&inst.dll_path) {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL patched", inst.version)).ok();
                tx.send(format!("..  {}  patching exe...", inst.version))
                    .ok();
                match core::patch_exe(&inst.exe_path) {
                    Ok(()) => {
                        if core::is_exe_patched(&inst.exe_path) {
                            tx.send(format!("..  {}  exe patched", inst.version)).ok();
                        } else {
                            tx.send(format!("..  {}  exe skipped (no cert)", inst.version))
                                .ok();
                        }
                    }
                    Err(e) => {
                        tx.send(format!("..  {}  exe patch failed: {}", inst.version, e))
                            .ok();
                    }
                }
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", inst.version, e))
                    .ok();
            }
        }
    }

    tx.send("..  patching shared licensing client...".into())
        .ok();
    match core::patch_shared_licensing_client() {
        Ok(()) => {
            if core::is_shared_licensing_client_patched() {
                tx.send("OK  shared licensing client patched".into()).ok();
            } else {
                tx.send("..  shared licensing client skipped (not found)".into())
                    .ok();
            }
        }
        Err(e) => {
            tx.send(format!("..  shared licensing client failed: {}", e))
                .ok();
        }
    }

    for (hub_idx, hub) in targets.hubs.iter().enumerate() {
        if !*targets.hub_selected.get(hub_idx).unwrap_or(&false) {
            continue;
        }
        if hub::is_hub_patched(&hub.asar_path) {
            tx.send(format!(
                "TOK  {} {}  already patched, skipped",
                hub.name, hub.version
            ))
            .ok();
            ok = true;
        } else {
            tx.send(format!("..  {} {}  patching...", hub.name, hub.version))
                .ok();
            match hub::patch_hub(hub) {
                Ok(()) => {
                    tx.send(format!("TOK  {} {}  success", hub.name, hub.version))
                        .ok();
                    ok = true;
                }
                Err(e) => {
                    tx.send(format!("TERR {} {}: {}", hub.name, hub.version, e))
                        .ok();
                }
            }
        }
    }

    if !ok {
        tx.send("ERR no target patched successfully".into()).ok();
    } else {
        deploy_license_and_hosts(&tx);
        tx.send("OK  patch complete".into()).ok();
    }
    tx.send(String::new()).ok();
}

fn deploy_license_and_hosts(tx: &mpsc::Sender<String>) {
    tx.send("..  generating license...".into()).ok();
    match core::sign_license() {
        Ok(xml) => match core::deploy_license(&xml) {
            Ok(()) => {
                tx.send("OK  license deployed".into()).ok();
            }
            Err(e) => {
                tx.send(format!("ERR deploy: {}", e)).ok();
            }
        },
        Err(e) => {
            tx.send(format!("ERR sign: {}", e)).ok();
        }
    }
    tx.send("..  writing hosts...".into()).ok();
    match core::write_hosts_if_needed() {
        Ok(()) => {
            tx.send("OK  hosts updated".into()).ok();
        }
        Err(e) => {
            tx.send(format!("ERR hosts: {}", e)).ok();
        }
    }
}

fn run_unpatch(tx: mpsc::Sender<String>, targets: JobTargets) {
    let mut ok = false;

    for inst in &targets.selected {
        tx.send(format!("..  {}  un-patching...", inst.version))
            .ok();
        match core::unpatch_dll(&inst.dll_path) {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL restored", inst.version)).ok();
                match core::unpatch_exe(&inst.exe_path) {
                    Ok(()) => {
                        tx.send(format!("..  {}  exe restored/skipped", inst.version))
                            .ok();
                    }
                    Err(e) => {
                        tx.send(format!("..  {}  exe restore failed: {}", inst.version, e))
                            .ok();
                    }
                }
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", inst.version, e))
                    .ok();
            }
        }
    }

    tx.send("..  un-patching shared licensing client...".into())
        .ok();
    match core::unpatch_shared_licensing_client() {
        Ok(()) => {
            tx.send("OK  shared licensing client restored".into()).ok();
        }
        Err(e) => {
            tx.send(format!("..  shared licensing client restore failed: {}", e))
                .ok();
        }
    }

    for (hub_idx, hub) in targets.hubs.iter().enumerate() {
        if !*targets.hub_selected.get(hub_idx).unwrap_or(&false) {
            continue;
        }
        tx.send(format!("..  {} {}  un-patching...", hub.name, hub.version))
            .ok();
        match hub::unpatch_hub(hub) {
            Ok(()) => {
                tx.send(format!("TOK  {} {}  restored", hub.name, hub.version))
                    .ok();
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {} {}: {}", hub.name, hub.version, e))
                    .ok();
            }
        }
    }

    if !ok {
        tx.send("ERR no target un-patched successfully".into()).ok();
    } else {
        cleanup_globals_if_no_targets_remain(&tx, &targets);
        tx.send("OK  un-patch complete".into()).ok();
    }
    tx.send(String::new()).ok();
}

fn cleanup_globals_if_no_targets_remain(tx: &mpsc::Sender<String>, targets: &JobTargets) {
    let any_editor_left = targets.all_paths.iter().any(|p| core::is_patched(p))
        || targets
            .all_exe_paths
            .iter()
            .any(|p| core::is_exe_patched(p))
        || core::is_shared_licensing_client_patched();
    let any_hub_left = targets
        .hubs
        .iter()
        .any(|h| hub::is_hub_patched(&h.asar_path));

    if !any_editor_left && !any_hub_left {
        tx.send("..  removing license...".into()).ok();
        match core::remove_license() {
            Ok(()) => {
                tx.send("OK  license removed".into()).ok();
            }
            Err(e) => {
                tx.send(format!("ERR remove license: {}", e)).ok();
            }
        }
        tx.send("..  cleaning hosts...".into()).ok();
        match core::clean_hosts() {
            Ok(()) => {
                tx.send("OK  hosts cleaned".into()).ok();
                tx.send("META cleaned-globals".into()).ok();
            }
            Err(e) => {
                tx.send(format!("ERR clean hosts: {}", e)).ok();
            }
        }
    } else {
        tx.send("OK  other patched targets remain, keeping license and hosts".into())
            .ok();
    }
}
