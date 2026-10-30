#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use wavical_ipc::{GrantTable, MAX_GRANTS_PER_WINDOW};

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }

    let table = GrantTable::new();
    let mut known_grants_win_a: Vec<String> = Vec::new();
    let mut known_grants_win_b: Vec<String> = Vec::new();

    let win_a = "window-alpha";
    let win_b = "window-beta";

    // Interpret chunks of fuzzed input as sequence of operations
    for chunk in data.chunks(8) {
        let op = chunk[0] % 4;
        let target_win = if (chunk[0] & 4) == 0 { win_a } else { win_b };
        let path_suffix = format!("file_{}", chunk.get(1).copied().unwrap_or(0));
        let path = PathBuf::from("C:\\test\\data").join(path_suffix);

        match op {
            0 => {
                // Insert grant
                match table.insert(target_win, path) {
                    Ok(grant_id) => {
                        if target_win == win_a {
                            known_grants_win_a.push(grant_id);
                        } else {
                            known_grants_win_b.push(grant_id);
                        }
                    }
                    Err(wavical_ipc::GrantError::LimitReached) => {
                        // Enforce capacity invariant
                        assert_eq!(table.count_for_window(target_win), MAX_GRANTS_PER_WINDOW);
                    }
                    Err(_) => {}
                }
            }
            1 => {
                // Lookup grant
                let grant_id = if !known_grants_win_a.is_empty() && (chunk[0] & 8) == 0 {
                    let idx = (chunk.get(2).copied().unwrap_or(0) as usize) % known_grants_win_a.len();
                    &known_grants_win_a[idx]
                } else if !known_grants_win_b.is_empty() {
                    let idx = (chunk.get(2).copied().unwrap_or(0) as usize) % known_grants_win_b.len();
                    &known_grants_win_b[idx]
                } else {
                    "00000000-0000-0000-0000-000000000000"
                };

                // Cross-window security invariant: Window A must never access Window B's grants
                if known_grants_win_b.contains(&grant_id.to_string()) {
                    assert!(
                        table.get(grant_id, win_a).is_err(),
                        "Cross-window grant access violation detected! Window A accessed Window B's grant"
                    );
                }

                if known_grants_win_a.contains(&grant_id.to_string()) {
                    assert!(
                        table.get(grant_id, win_b).is_err(),
                        "Cross-window grant access violation detected! Window B accessed Window A's grant"
                    );
                }
            }
            2 => {
                // Remove grant
                if !known_grants_win_a.is_empty() && target_win == win_a {
                    let idx = (chunk.get(2).copied().unwrap_or(0) as usize) % known_grants_win_a.len();
                    let gid = known_grants_win_a.remove(idx);
                    let _ = table.remove_grant(&gid, win_a);
                    assert!(table.get(&gid, win_a).is_err());
                }
            }
            3 => {
                // Clear window
                table.clear_window(target_win);
                assert_eq!(table.count_for_window(target_win), 0);
                if target_win == win_a {
                    for gid in &known_grants_win_a {
                        assert!(table.get(gid, win_a).is_err());
                    }
                    known_grants_win_a.clear();
                } else {
                    for gid in &known_grants_win_b {
                        assert!(table.get(gid, win_b).is_err());
                    }
                    known_grants_win_b.clear();
                }
            }
            _ => unreachable!(),
        }

        // Table size invariant: count for any window never exceeds 64
        assert!(table.count_for_window(win_a) <= MAX_GRANTS_PER_WINDOW);
        assert!(table.count_for_window(win_b) <= MAX_GRANTS_PER_WINDOW);
    }
});
