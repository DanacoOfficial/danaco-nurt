// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-proc/tests/track_classify.rs
// Responsibility:   Classification of command lines by external effect and by behaviour.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Classification of command lines by external effect and by behaviour.

use nurt_contracts::error::ErrorCode;
use nurt_proc::{EffectClassifier, classify_argv, classify_effects};

/// Smallest number of cases per direction required of the effect table.
const MIN_CASES: usize = 40;

fn words(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// Command lines with an effect outside the computer and the expected rule.
const EFFECTS: &[(&str, &str)] = &[
    ("git push origin main", "git push"),
    ("git -C repo push --force", "git push"),
    ("/usr/bin/git -c user.name=x push", "git push"),
    ("git send-email patches/", "git send-email|request-pull"),
    ("docker push registry/app:1", "docker|podman|nerdctl push"),
    ("podman push app", "docker|podman|nerdctl push"),
    (
        "docker --context prod image push app",
        "docker|podman|nerdctl image|manifest push",
    ),
    (
        "docker buildx build --push -t app .",
        "docker|podman buildx build --push",
    ),
    (
        "npm publish --access public",
        "npm|pnpm publish|unpublish|deprecate",
    ),
    ("pnpm publish", "npm|pnpm publish|unpublish|deprecate"),
    ("yarn npm publish", "yarn npm publish|tag"),
    ("cargo publish -p nurt", "cargo publish|yank|owner"),
    ("twine upload dist/*", "twine upload"),
    ("python3 -m twine upload dist/*", "twine upload"),
    ("gem push app.gem", "gem push|yank|owner"),
    (
        "kubectl apply -f deploy.yaml",
        "kubectl|oc apply|delete|create|replace|patch|scale|edit|label|annotate|set|expose|run|drain|cordon|uncordon|taint|autoscale",
    ),
    (
        "kubectl -n prod delete pod web-1",
        "kubectl|oc apply|delete|create|replace|patch|scale|edit|label|annotate|set|expose|run|drain|cordon|uncordon|taint|autoscale",
    ),
    (
        "kubectl rollout restart deploy/web",
        "kubectl|oc rollout restart|undo|pause|resume",
    ),
    (
        "helm upgrade --install web chart/",
        "helm install|upgrade|uninstall|delete|rollback|push",
    ),
    (
        "terraform apply -auto-approve",
        "terraform|tofu apply|destroy|import|taint|untaint",
    ),
    (
        "terraform -chdir=infra destroy",
        "terraform|tofu apply|destroy|import|taint|untaint",
    ),
    (
        "tofu apply",
        "terraform|tofu apply|destroy|import|taint|untaint",
    ),
    (
        "pulumi up --yes",
        "pulumi up|update|destroy|import|refresh|cancel",
    ),
    (
        "aws --region eu-west-1 ec2 terminate-instances --instance-ids i-1",
        "aws * create*|delete*|put-*|update-*|run-*|terminate-*|start-*|stop-*|reboot-*|modify-*|attach-*|detach-*|associate-*|disassociate-*|register-*|deregister-*|deploy|invoke|publish|send-*|tag-*|untag-*|remove-*|add-*|set-*|import-*|restore-*|reset-*|enable-*|disable-*|cancel-*|copy-*|upload-*|replace-*|revoke-*|authorize-*|change-*|apply-*",
    ),
    (
        "aws s3 sync build/ s3://bucket",
        "aws s3 cp|mv|rm|sync|mb|rb|website",
    ),
    (
        "gcloud compute instances delete vm-1",
        "gcloud ** create|delete|update|deploy|patch|start|stop|reset|resize|import|submit|add-*|remove-*|set-iam-policy|enable|disable|publish|undelete|restore|rollback",
    ),
    (
        "az group create -n rg -l westeurope",
        "az ** create|delete|update|set|start|stop|restart|deploy|add|remove|import|purge|upload|upload-batch|delete-batch|swap|scale|resize|assign|grant|revoke",
    ),
    (
        "curl -X POST https://api.example.com/items",
        "curl -X|--request=POST|PUT|DELETE|PATCH",
    ),
    (
        "curl -sXDELETE https://api.example.com/items/1",
        "curl -X|--request=POST|PUT|DELETE|PATCH",
    ),
    (
        "curl --request=put https://api.example.com/items/1",
        "curl -X|--request=POST|PUT|DELETE|PATCH",
    ),
    (
        "curl -d name=x https://api.example.com/items",
        "curl -d|--data*|--json|-F|--form*|-T|--upload-file",
    ),
    (
        "curl --data-binary @file https://api.example.com",
        "curl -d|--data*|--json|-F|--form*|-T|--upload-file",
    ),
    (
        "wget --post-data=a=1 https://example.com",
        "wget --post-data*|--post-file*|--body-data*|--body-file*",
    ),
    ("psql -c 'delete from t' db", "psql|pgcli"),
    ("mysql -e 'drop table t'", "mysql|mariadb|mycli"),
    ("mongosh --eval 'db.x.drop()'", "mongosh|mongo"),
    ("redis-cli flushall", "redis-cli|valkey-cli"),
    ("ssh host uptime", "ssh"),
    ("scp file host:/tmp/", "scp|sftp"),
    (
        "rsync -av --exclude target ./ backup:/srv/app",
        "rsync remote",
    ),
    (
        "gh pr merge 12 --squash",
        "gh pr merge|create|close|comment|edit|review|ready|reopen",
    ),
    (
        "gh release create v1.0.0",
        "gh release create|delete|upload|edit",
    ),
    (
        "systemctl restart nginx",
        "systemctl start|stop|restart|reload|try-restart|reload-or-restart|enable|disable|mask|unmask|kill|isolate|daemon-reload|poweroff|reboot|halt|suspend|hibernate",
    ),
    (
        "sudo -u root systemctl --user stop app",
        "systemctl start|stop|restart|reload|try-restart|reload-or-restart|enable|disable|mask|unmask|kill|isolate|daemon-reload|poweroff|reboot|halt|suspend|hibernate",
    ),
    (
        "env AWS_PROFILE=prod nice -n 5 terraform apply",
        "terraform|tofu apply|destroy|import|taint|untaint",
    ),
    (
        "time -p timeout 60 cargo publish",
        "cargo publish|yank|owner",
    ),
    (
        "npx vercel@latest deploy --prod",
        "vercel|netlify|firebase|wrangler|fly|flyctl|serverless|sls|cdk|sam|eb deploy|publish|destroy|remove|terminate|delete",
    ),
    (
        "pnpm exec wrangler deploy",
        "vercel|netlify|firebase|wrangler|fly|flyctl|serverless|sls|cdk|sam|eb deploy|publish|destroy|remove|terminate|delete",
    ),
    (
        "bunx netlify deploy",
        "vercel|netlify|firebase|wrangler|fly|flyctl|serverless|sls|cdk|sam|eb deploy|publish|destroy|remove|terminate|delete",
    ),
    ("uvx twine upload dist/x.whl", "twine upload"),
    ("C:\\Tools\\git.exe push", "git push"),
    (
        "sudo apt-get install -y curl",
        "apt|apt-get|aptitude install|remove|purge|upgrade|dist-upgrade|full-upgrade|autoremove|reinstall",
    ),
    (
        "mvn -B clean deploy",
        "mvn|mvnw ** deploy|release:perform|deploy:deploy-file",
    ),
    ("ansible-playbook site.yml", "ansible-playbook"),
    ("kill -9 1234", "kill|pkill|killall|taskkill"),
];

/// Command lines without an effect outside the computer.
const NO_EFFECTS: &[&str] = &[
    "git status",
    "git log --grep push",
    "git commit -m push",
    "git fetch origin",
    "git pull",
    "docker build -t app .",
    "docker run --rm app",
    "docker pull alpine",
    "npm install",
    "npm run build",
    "npm access list packages",
    "pnpm test",
    "cargo build --release",
    "cargo test",
    "twine check dist/*",
    "python -m pytest",
    "python3 script.py push",
    "kubectl get pods",
    "kubectl -n prod describe pod web-1",
    "kubectl rollout status deploy/web",
    "kubectl logs -f web-1",
    "helm template chart/",
    "helm list",
    "terraform plan",
    "terraform init",
    "tofu validate",
    "pulumi preview",
    "aws s3 ls s3://bucket",
    "aws ec2 describe-instances",
    "gcloud compute instances list",
    "az group list",
    "curl https://example.com",
    "curl -sSL https://example.com/install.sh",
    "curl -X GET https://api.example.com",
    "curl -D headers.txt https://example.com",
    "wget https://example.com/file.tar.gz",
    "rsync -av src/ dst/",
    "gh pr list",
    "gh pr view 12",
    "systemctl status nginx",
    "systemctl --user list-units",
    "ls -la",
    "make test",
    "sudo -l",
    "env FOO=1 make",
    "ssh -V",
    "vercel --version",
    "brew services list",
    "pacman -Ss curl",
    "vault kv get secret/app",
    "rg push src",
    "echo git push",
];

#[test]
fn effect_table_matches_programs_with_external_effects() {
    assert!(EFFECTS.len() >= MIN_CASES);
    for (line, rule) in EFFECTS {
        let matches = classify_effects(&words(line));
        assert!(
            matches.iter().any(|found| found.rule == *rule),
            "{line}: expected `{rule}`, found {matches:?}"
        );
    }
}

#[test]
fn effect_table_leaves_other_programs_unmarked() {
    assert!(NO_EFFECTS.len() >= MIN_CASES);
    for line in NO_EFFECTS {
        let matches = classify_effects(&words(line));
        assert!(matches.is_empty(), "{line}: unexpected {matches:?}");
    }
}

#[test]
fn effect_match_names_the_program_behind_wrappers() {
    let matches = classify_effects(&words("sudo env A=1 npx cargo publish"));
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].program, "cargo");
    let matches = classify_effects(&words("python3 -m twine upload x"));
    assert_eq!(matches[0].program, "twine");
}

#[test]
fn configured_patterns_extend_the_table() {
    let classifier =
        EffectClassifier::with_patterns(&["deploy-tool publish|release", "notify * --channel=ops"])
            .unwrap();
    assert_eq!(
        classifier.classify(&words("deploy-tool release v2"))[0].rule,
        "deploy-tool publish|release"
    );
    assert_eq!(
        classifier.classify(&words("notify build --channel ops"))[0].rule,
        "notify * --channel=ops"
    );
    assert!(
        classifier
            .classify(&words("notify build --channel dev"))
            .is_empty()
    );
    assert!(classifier.classify(&words("deploy-tool status")).is_empty());
    assert_eq!(classifier.classify(&words("git push"))[0].rule, "git push");
}

#[test]
fn rejects_malformed_configured_patterns() {
    for pattern in ["", "--force push", "tool a||b"] {
        let error = EffectClassifier::with_patterns(&[pattern]).err().unwrap();
        assert_eq!(error.code(), ErrorCode::Config, "{pattern:?}");
    }
}

#[test]
fn waiting_programs_are_classified() {
    for line in [
        "sleep 30",
        "tail -f app.log",
        "tail -n 100 -F app.log",
        "tail --follow=name app.log",
        "watch -n 1 date",
        "journalctl -u app -f",
        "kubectl logs -f web-1",
        "docker logs --follow app",
        "docker compose up",
        "ping example.com",
        "top -b",
        "timeout 60 tail -f app.log",
        "python3 -m http.server 8000",
    ] {
        assert!(classify_argv(&words(line)).waits.is_some(), "{line}");
    }
    for line in [
        "tail -n 20 app.log",
        "journalctl -u app -n 50",
        "kubectl logs web-1",
        "docker compose up -d",
        "ping -c 3 example.com",
        "top -b -n 1",
        "sleepy",
    ] {
        assert!(classify_argv(&words(line)).waits.is_none(), "{line}");
    }
}

#[test]
fn interactive_programs_are_classified() {
    for line in [
        "vim notes.txt",
        "less app.log",
        "top",
        "htop",
        "ssh host",
        "ssh -p 2222 host",
        "git rebase -i HEAD~3",
        "git add -p",
        "git commit",
        "docker exec -it app sh",
        "kubectl exec -ti web -- bash",
        "crontab -e",
        "sudo -i",
        "tmux",
    ] {
        let classes = classify_argv(&words(line));
        let interactive = classes.interactive.unwrap_or_else(|| panic!("{line}"));
        assert!(!interactive.without_input, "{line}");
    }
    for line in ["python3", "node", "psql db", "bash", "sqlite3 app.db"] {
        let interactive = classify_argv(&words(line))
            .interactive
            .unwrap_or_else(|| panic!("{line}"));
        assert!(interactive.without_input, "{line}");
    }
    for line in [
        "vim --version",
        "top -b -n 1",
        "ssh host uptime",
        "git commit -m message",
        "git commit -am message",
        "git rebase main",
        "docker exec app ls",
        "python3 script.py",
        "python3 -c pass",
        "psql -c 'select 1' db",
        "mysql -e 'select 1'",
        "sqlite3 app.db 'select 1'",
        "tmux ls",
        "tmux new-session -d -s build",
        "crontab -l",
    ] {
        assert!(classify_argv(&words(line)).interactive.is_none(), "{line}");
    }
}

#[test]
fn opaque_scripts_are_extracted() {
    let cases: &[(&[&str], &str, &str)] = &[
        (
            &["bash", "-c", "make && make test"],
            "bash",
            "make && make test",
        ),
        (&["sh", "-ec", "set -x; ls"], "sh", "set -x; ls"),
        (
            &["sudo", "sh", "-c", "echo 1 > /proc/x"],
            "sh",
            "echo 1 > /proc/x",
        ),
        (
            &[
                "pwsh",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-ChildItem",
                "-Recurse",
            ],
            "pwsh",
            "Get-ChildItem -Recurse",
        ),
        (&["cmd", "/d", "/c", "dir", "/s"], "cmd", "dir /s"),
        (&["python3", "-c", "print(1)"], "python3", "print(1)"),
        (&["node", "-e", "console.log(1)"], "node", "console.log(1)"),
        (&["perl", "-e", "print 1"], "perl", "print 1"),
        (
            &["deno", "eval", "console.log(1)"],
            "deno",
            "console.log(1)",
        ),
        (
            &["ssh", "host", "rm", "-rf", "/tmp/x"],
            "ssh",
            "rm -rf /tmp/x",
        ),
    ];
    for (argv, interpreter, script) in cases {
        let opaque = classify_argv(argv)
            .opaque_script
            .unwrap_or_else(|| panic!("{argv:?}"));
        assert_eq!(opaque.interpreter, *interpreter);
        assert_eq!(opaque.script, *script);
    }
    for argv in [
        &["bash", "script.sh", "-c", "x"][..],
        &["pwsh", "-File", "build.ps1"],
        &["python3", "tool.py", "-c", "x"],
        &["ssh", "host"],
        &["node", "server.js"],
    ] {
        assert!(classify_argv(argv).opaque_script.is_none(), "{argv:?}");
    }
}

#[test]
fn empty_command_line_has_no_class_and_no_effect() {
    let empty: [&str; 0] = [];
    assert_eq!(classify_argv(&empty), nurt_proc::ArgvClasses::default());
    assert!(classify_effects(&empty).is_empty());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
