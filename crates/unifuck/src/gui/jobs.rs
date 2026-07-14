use crate::core;
use crate::core::EditorInfo;
use crate::gui::app::AppState;
use crate::hub::{self, HubInfo};
use crate::vendor::Vendor;
use std::collections::HashSet;
use std::sync::mpsc;
use std::thread;

struct JobTargets {
    selected_editors: Vec<EditorInfo>,
    all_editors: Vec<EditorInfo>,
    selected_hubs: Vec<HubInfo>,
    all_hubs: Vec<HubInfo>,
    patch_shared: bool,
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

    let mut selected_editors = Vec::new();
    let mut selected_hubs = Vec::new();
    let mut all_editors = Vec::new();
    let mut all_hubs = Vec::new();
    let mut patch_shared = false;

    for (group_idx, group) in app.groups.iter().enumerate() {
        if app.group_selected.get(group_idx).copied().unwrap_or(false) {
            selected_hubs.push(group.hub.clone());
            patch_shared = true;
            for editor in &group.editors {
                selected_editors.push(editor.clone());
            }
        }
        for editor in &group.editors {
            all_editors.push(editor.clone());
        }
        all_hubs.push(group.hub.clone());
    }

    if selected_editors.is_empty() && selected_hubs.is_empty() && !patch_shared {
        return None;
    }

    Some(JobTargets {
        selected_editors,
        all_editors,
        selected_hubs,
        all_hubs,
        patch_shared,
        is_unpatch: mode,
    })
}

fn run_patch(tx: mpsc::Sender<String>, targets: JobTargets) {
    let mut ok = false;

    for inst in &targets.selected_editors {
        let label = format!("{} {}", inst.vendor.display_name(), inst.version);
        if core::is_patched(inst) {
            tx.send(format!("TOK  {}  already patched, skipped", label))
                .ok();
            ok = true;
            continue;
        }
        tx.send(format!("..  {}  patching DLL...", label)).ok();
        match core::patch_dll(inst.vendor, &inst.dll_path()) {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL patched", label)).ok();
                tx.send(format!("..  {}  patching exe...", label)).ok();
                match core::patch_exe(&inst.exe_path()) {
                    Ok(()) => {
                        if core::is_exe_patched(&inst.exe_path()) {
                            tx.send(format!("..  {}  exe patched", label)).ok();
                        } else {
                            tx.send(format!("..  {}  exe skipped (no cert)", label))
                                .ok();
                        }
                    }
                    Err(e) => {
                        tx.send(format!("..  {}  exe patch failed: {}", label, e))
                            .ok();
                    }
                }
                if inst.vendor.supports_native_hook() {
                    tx.send(format!("..  {}  deploying loader...", label)).ok();
                    match core::deploy_loader(&inst.root) {
                        Ok(()) => {
                            tx.send(format!("..  {}  loader deployed", label)).ok();
                        }
                        Err(e) => {
                            tx.send(format!("..  {}  loader deploy failed: {}", label, e))
                                .ok();
                        }
                    }
                }
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", label, e)).ok();
            }
        }
    }

    if targets.patch_shared {
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
    }

    for hub in &targets.selected_hubs {
        let label = format!("{} {}", hub.vendor.hub_name(), hub.version);
        if hub::is_hub_patched(&hub.root) {
            tx.send(format!("TOK  {}  already patched, skipped", label))
                .ok();
            ok = true;
        } else {
            tx.send(format!("..  {}  patching...", label)).ok();
            match hub::patch_hub(hub) {
                Ok(()) => {
                    tx.send(format!("TOK  {}  success", label)).ok();
                    ok = true;
                }
                Err(e) => {
                    tx.send(format!("TERR {}: {}", label, e)).ok();
                }
            }
        }
    }

    if !ok {
        tx.send("ERR no target patched successfully".into()).ok();
    } else {
        deploy_license_and_hosts(&tx, &targets);
        tx.send("OK  patch complete".into()).ok();
    }
    tx.send(String::new()).ok();
}

fn vendors_touched(targets: &JobTargets) -> Vec<Vendor> {
    let mut set: HashSet<Vendor> = HashSet::new();
    for inst in &targets.selected_editors {
        set.insert(inst.vendor);
    }
    for hub in &targets.selected_hubs {
        set.insert(hub.vendor);
    }
    Vendor::all()
        .iter()
        .copied()
        .filter(|v| set.contains(v))
        .collect()
}

fn deploy_license_and_hosts(tx: &mpsc::Sender<String>, targets: &JobTargets) {
    for vendor in vendors_touched(targets) {
        let v = vendor.display_name();
        tx.send(format!("..  generating {} license...", v)).ok();
        match core::sign_license(vendor) {
            Ok(xml) => match core::deploy_license(vendor, &xml) {
                Ok(()) => {
                    tx.send(format!("OK  {} license deployed", v)).ok();
                }
                Err(e) => {
                    tx.send(format!("ERR deploy {} license: {}", v, e)).ok();
                }
            },
            Err(e) => {
                tx.send(format!("ERR sign {} license: {}", v, e)).ok();
            }
        }
    }
    tx.send("..  writing hosts...".into()).ok();
    match core::deploy_hosts() {
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

    for inst in &targets.selected_editors {
        let label = format!("{} {}", inst.vendor.display_name(), inst.version);
        tx.send(format!("..  {}  un-patching...", label)).ok();
        match core::unpatch_dll(&inst.dll_path()) {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL restored", label)).ok();
                match core::unpatch_exe(&inst.exe_path()) {
                    Ok(()) => {
                        tx.send(format!("..  {}  exe restored/skipped", label)).ok();
                    }
                    Err(e) => {
                        tx.send(format!("..  {}  exe restore failed: {}", label, e))
                            .ok();
                    }
                }
                if inst.vendor.supports_native_hook() {
                    tx.send(format!("..  {}  reverting loader...", label)).ok();
                    match core::revert_loader(&inst.root) {
                        Ok(()) => {
                            tx.send(format!("..  {}  loader reverted", label)).ok();
                        }
                        Err(e) => {
                            tx.send(format!("..  {}  loader revert failed: {}", label, e))
                                .ok();
                        }
                    }
                }
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", label, e)).ok();
            }
        }
    }

    if targets.patch_shared {
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
    }

    for hub in &targets.selected_hubs {
        let label = format!("{} {}", hub.vendor.hub_name(), hub.version);
        tx.send(format!("..  {}  un-patching...", label)).ok();
        match hub::unpatch_hub(hub) {
            Ok(()) => {
                tx.send(format!("TOK  {}  restored", label)).ok();
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}: {}", label, e)).ok();
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
    let shared_left = core::is_shared_licensing_client_patched();
    let mut anything_left = shared_left;

    for vendor in Vendor::all().iter().copied() {
        let editor_left = targets
            .all_editors
            .iter()
            .filter(|i| i.vendor == vendor)
            .any(|i| core::is_patched(i) || core::is_exe_patched(&i.exe_path()));
        let hub_left = targets
            .all_hubs
            .iter()
            .filter(|h| h.vendor == vendor)
            .any(|h| hub::is_hub_patched(&h.root));

        if !editor_left && !hub_left {
            let v = vendor.display_name();
            tx.send(format!("..  removing {} license...", v)).ok();
            match core::remove_license(vendor) {
                Ok(()) => {
                    tx.send(format!("OK  {} license removed", v)).ok();
                }
                Err(e) => {
                    tx.send(format!("ERR remove {} license: {}", v, e)).ok();
                }
            }
        } else {
            anything_left = true;
        }
    }

    if !anything_left {
        tx.send("..  cleaning hosts...".into()).ok();
        match core::remove_hosts() {
            Ok(()) => {
                tx.send("OK  hosts cleaned".into()).ok();
                tx.send("META cleaned-globals".into()).ok();
            }
            Err(e) => {
                tx.send(format!("ERR clean hosts: {}", e)).ok();
            }
        }
    } else {
        tx.send("OK  other patched targets remain, keeping hosts".into())
            .ok();
    }
}
