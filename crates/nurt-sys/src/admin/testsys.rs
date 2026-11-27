// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! An in-memory Linux for the tests of the administration commands: it
//! answers the reading programs and applies the changing programs that the
//! commands build, so that a command and its undo run against one state.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use nurt_contracts::env::Os;
use nurt_contracts::error::Result;

use super::sys::{Call, Output, Sys};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct User {
    pub uid: u32,
    pub shell: String,
    pub home: String,
    pub comment: String,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Group {
    pub gid: u32,
    pub members: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct State {
    pub users: BTreeMap<String, User>,
    pub groups: BTreeMap<String, Group>,
    pub mounts: BTreeMap<String, (String, String, String)>,
    pub sysctl: BTreeMap<String, String>,
    pub modules: BTreeSet<String>,
    pub files: BTreeMap<String, String>,
    pub schedule: Option<String>,
    pub passwords: BTreeMap<String, String>,
    pub lsblk: String,
    pub formatted: Vec<String>,
}

/// The in-memory system and the log of the programs it ran.
pub(crate) struct FakeSys {
    os: Os,
    pub state: Mutex<State>,
    pub log: Mutex<Vec<Call>>,
    now: i128,
}

fn done(code: i32, stdout: &str, stderr: &str) -> Result<Output> {
    Ok(Output {
        code: Some(code),
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    })
}

impl FakeSys {
    pub(crate) fn linux() -> Self {
        let mut state = State::default();
        state.users.insert(
            "root".to_owned(),
            User {
                uid: 0,
                shell: "/bin/bash".to_owned(),
                home: "/root".to_owned(),
                comment: "root".to_owned(),
                locked: false,
            },
        );
        state.groups.insert(
            "root".to_owned(),
            Group {
                gid: 0,
                members: BTreeSet::new(),
            },
        );
        for (name, gid) in [("wheel", 10), ("docker", 998)] {
            state.groups.insert(
                name.to_owned(),
                Group {
                    gid,
                    members: BTreeSet::new(),
                },
            );
        }
        state
            .sysctl
            .insert("net.ipv4.ip_forward".to_owned(), "0".to_owned());
        state
            .sysctl
            .insert("vm.swappiness".to_owned(), "60".to_owned());
        state.modules.insert("loop".to_owned());
        Self {
            os: Os::Linux,
            state: Mutex::new(state),
            log: Mutex::new(Vec::new()),
            now: 1_700_000_000 * 1_000_000_000,
        }
    }

    pub(crate) fn with_os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    pub(crate) fn snapshot(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub(crate) fn restore(&self, state: State) {
        *self.state.lock().unwrap() = state;
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(Call::line).collect()
    }

    pub(crate) fn set_file(&self, path: &str, text: &str) {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(path.to_owned(), text.to_owned());
    }

    pub(crate) fn file(&self, path: &str) -> Option<String> {
        self.state.lock().unwrap().files.get(path).cloned()
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1).cloned())
}

impl Sys for FakeSys {
    fn os(&self) -> Os {
        self.os
    }

    fn read_file(&self, path: &str) -> Result<Option<String>> {
        Ok(self.state.lock().unwrap().files.get(path).cloned())
    }

    fn now_nanos(&self) -> i128 {
        self.now
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per program of the fake system"
    )]
    fn run(&self, call: &Call) -> Result<Output> {
        self.log.lock().unwrap().push(call.clone());
        let mut state = self.state.lock().unwrap();
        let args = &call.args;
        let last = args.last().cloned().unwrap_or_default();
        match call.program.as_str() {
            "getent" => {
                let name = &args[1];
                if args[0] == "passwd" {
                    return match state.users.get(name) {
                        Some(user) => {
                            let gid = state.groups.get(name).map_or(user.uid, |g| g.gid);
                            done(
                                0,
                                &format!(
                                    "{name}:x:{}:{gid}:{}:{}:{}\n",
                                    user.uid, user.comment, user.home, user.shell
                                ),
                                "",
                            )
                        }
                        None => done(2, "", ""),
                    };
                }
                match state.groups.get(name) {
                    Some(group) => {
                        let members: Vec<&str> = group.members.iter().map(String::as_str).collect();
                        done(
                            0,
                            &format!("{name}:x:{}:{}\n", group.gid, members.join(",")),
                            "",
                        )
                    }
                    None => done(2, "", ""),
                }
            }
            "id" => {
                let name = &args[1];
                if !state.users.contains_key(name) {
                    return done(1, "", "id: no such user");
                }
                let primary = if state.groups.contains_key(name) {
                    name.clone()
                } else {
                    "users".to_owned()
                };
                if args[0] == "-gn" {
                    return done(0, &format!("{primary}\n"), "");
                }
                let mut all = vec![primary];
                all.extend(
                    state
                        .groups
                        .iter()
                        .filter(|(_, group)| group.members.contains(name))
                        .map(|(group, _)| group.clone()),
                );
                done(0, &format!("{}\n", all.join(" ")), "")
            }
            "passwd" => match state.users.get(&last) {
                Some(user) => done(
                    0,
                    &format!(
                        "{last} {} 2020-01-01 0 99999 7 -1\n",
                        if user.locked { "L" } else { "P" }
                    ),
                    "",
                ),
                None => done(1, "", "unknown user"),
            },
            "useradd" => {
                let name = last;
                if state.users.contains_key(&name) {
                    return done(9, "", "useradd: user exists");
                }
                let uid = flag_value(args, "-u")
                    .and_then(|u| u.parse().ok())
                    .unwrap_or_else(|| {
                        state
                            .users
                            .values()
                            .map(|u| u.uid)
                            .max()
                            .unwrap_or(999)
                            .max(999)
                            + 1
                    });
                let home = flag_value(args, "-d").unwrap_or_else(|| format!("/home/{name}"));
                state.users.insert(
                    name.clone(),
                    User {
                        uid,
                        shell: flag_value(args, "-s").unwrap_or_else(|| "/bin/sh".to_owned()),
                        home,
                        comment: flag_value(args, "-c").unwrap_or_default(),
                        locked: false,
                    },
                );
                state.groups.insert(
                    name.clone(),
                    Group {
                        gid: uid,
                        members: BTreeSet::new(),
                    },
                );
                if let Some(groups) = flag_value(args, "-G") {
                    for group in groups.split(',') {
                        match state.groups.get_mut(group) {
                            Some(found) => {
                                found.members.insert(name.clone());
                            }
                            None => return done(6, "", "useradd: group does not exist"),
                        }
                    }
                }
                done(0, "", "")
            }
            "usermod" => {
                let name = last;
                if !state.users.contains_key(&name) {
                    return done(6, "", "usermod: user does not exist");
                }
                if let Some(groups) = flag_value(args, "-G") {
                    for group in state.groups.values_mut() {
                        group.members.remove(&name);
                    }
                    for group in groups.split(',').filter(|g| !g.is_empty()) {
                        if let Some(found) = state.groups.get_mut(group) {
                            found.members.insert(name.clone());
                        }
                    }
                }
                let user = state.users.get_mut(&name).unwrap();
                if let Some(shell) = flag_value(args, "-s") {
                    user.shell = shell;
                }
                if let Some(home) = flag_value(args, "-d") {
                    user.home = home;
                }
                if let Some(comment) = flag_value(args, "-c") {
                    user.comment = comment;
                }
                if args.iter().any(|a| a == "-L") {
                    user.locked = true;
                }
                if args.iter().any(|a| a == "-U") {
                    user.locked = false;
                }
                done(0, "", "")
            }
            "userdel" => {
                if state.users.remove(&last).is_none() {
                    return done(6, "", "userdel: user does not exist");
                }
                state.groups.remove(&last);
                for group in state.groups.values_mut() {
                    group.members.remove(&last);
                }
                done(0, "", "")
            }
            "chpasswd" => {
                let text = call.stdin.clone().unwrap_or_default();
                if let Some((name, value)) = text.trim_end().split_once(':') {
                    state.passwords.insert(name.to_owned(), value.to_owned());
                }
                done(0, "", "")
            }
            "groupadd" => {
                if state.groups.contains_key(&last) {
                    return done(9, "", "groupadd: group exists");
                }
                let gid = flag_value(args, "-g")
                    .and_then(|g| g.parse().ok())
                    .unwrap_or_else(|| {
                        state
                            .groups
                            .values()
                            .map(|g| g.gid)
                            .max()
                            .unwrap_or(999)
                            .max(999)
                            + 1
                    });
                state.groups.insert(
                    last,
                    Group {
                        gid,
                        members: BTreeSet::new(),
                    },
                );
                done(0, "", "")
            }
            "groupdel" => match state.groups.remove(&last) {
                Some(_) => done(0, "", ""),
                None => done(6, "", "groupdel: group does not exist"),
            },
            "gpasswd" => {
                let (flag, user, group) = (&args[0], args[1].clone(), &args[2]);
                match state.groups.get_mut(group) {
                    Some(found) => {
                        if flag == "-a" {
                            found.members.insert(user);
                        } else {
                            found.members.remove(&user);
                        }
                        done(0, "", "")
                    }
                    None => done(3, "", "gpasswd: group does not exist"),
                }
            }
            "findmnt" => {
                let target = flag_value(args, "-M").unwrap_or_default();
                match state.mounts.get(&target) {
                    Some((device, fstype, options)) => {
                        done(0, &format!("{device} {fstype} {options}\n"), "")
                    }
                    None => done(1, "", ""),
                }
            }
            "mount" => {
                let target = args[args.len() - 1].clone();
                let device = args[args.len() - 2].clone();
                if state.mounts.contains_key(&target) {
                    return done(32, "", "mount: already mounted");
                }
                let fstype = flag_value(args, "-t").unwrap_or_else(|| "ext4".to_owned());
                let options = flag_value(args, "-o").unwrap_or_else(|| "rw,relatime".to_owned());
                state.mounts.insert(target, (device, fstype, options));
                done(0, "", "")
            }
            "umount" => match state.mounts.remove(&last) {
                Some(_) => done(0, "", ""),
                None => done(32, "", "umount: not mounted"),
            },
            "sysctl" => {
                if args[0] == "-n" {
                    return match state.sysctl.get(&args[1]) {
                        Some(value) => done(0, &format!("{value}\n"), ""),
                        None => done(255, "", "cannot stat"),
                    };
                }
                let (key, value) = args[1].split_once('=').unwrap();
                state.sysctl.insert(key.to_owned(), value.to_owned());
                done(0, "", "")
            }
            "lsmod" => {
                let mut text = "Module Size Used by\n".to_owned();
                for module in &state.modules {
                    text.push_str(&format!("{module} 16384 0\n"));
                }
                done(0, &text, "")
            }
            "modprobe" => {
                if args[0] == "-r" {
                    state.modules.remove(&args[1].replace('-', "_"));
                } else {
                    state.modules.insert(args[0].replace('-', "_"));
                }
                done(0, "", "")
            }
            "shutdown" => {
                if args[0] == "-c" {
                    state.schedule = None;
                } else if args[1] != "now" {
                    let mode = if args[0] == "-r" {
                        "reboot"
                    } else {
                        "poweroff"
                    };
                    let text = format!("USEC=1\nWARN_WALL=1\nMODE={mode}\n");
                    state.schedule = Some(text.clone());
                    state
                        .files
                        .insert("/run/systemd/shutdown/scheduled".to_owned(), text);
                }
                if args[0] == "-c" {
                    state.files.remove("/run/systemd/shutdown/scheduled");
                }
                done(0, "", "")
            }
            "lsblk" => done(0, &state.lsblk, ""),
            "parted" => done(0, "", ""),
            program if program.starts_with("mkfs.") => {
                state.formatted.push(call.line());
                done(0, "", "")
            }
            other => done(127, "", &format!("{other}: not found")),
        }
    }
}
