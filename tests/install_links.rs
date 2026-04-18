#![cfg(unix)]

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use agents_unified::install::{init_agents_home, install_project, InitOptions, InstallOptions};
use agents_unified::model::cursor_display_name;
use agents_unified::model::{AgentId, LinkKind, PlannedLink};
use agents_unified::plugins::{InstallContext, PluginRegistry, ProjectLinker};

#[test]
fn cursor_renames_md_to_mdc_for_display() {
    assert_eq!(cursor_display_name("foo.md"), "foo.mdc");
    assert_eq!(cursor_display_name("foo.mdc"), "foo.mdc");
}

struct DummyPlugin;

impl ProjectLinker for DummyPlugin {
    fn id(&self) -> &'static str {
        "dummy"
    }

    fn plan(&self, ctx: &InstallContext<'_>) -> Vec<PlannedLink> {
        let src = ctx.agents_home.join("local/note.txt");
        vec![PlannedLink {
            agent: AgentId::Cursor,
            kind: LinkKind::Symlink,
            source: src,
            dest: ctx.project_path.join("PLUGIN_NOTE.txt"),
        }]
    }
}

#[test]
fn install_creates_cursor_hardlinks_and_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let agents = tmp.path().join("agents");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    init_agents_home(&agents, &InitOptions::default()).unwrap();

    fs::write(
        agents.join("rules/global/hello.md"),
        b"---\n---\n\nhello\n",
    )
    .unwrap();
    fs::create_dir_all(agents.join("rules/p1")).unwrap();
    fs::write(agents.join("rules/p1/rules.md"), b"# rules\n").unwrap();
    fs::write(agents.join("settings/global/cursor.json"), b"{}\n").unwrap();
    fs::write(agents.join("mcp/global/cursor.json"), b"{}\n").unwrap();
    fs::create_dir_all(agents.join("skills/global/demo")).unwrap();
    fs::write(
        agents.join("skills/global/demo/SKILL.md"),
        b"---\nname: demo\n---\n",
    )
    .unwrap();
    fs::write(agents.join("local/note.txt"), b"plugin\n").unwrap();

    let mut reg = PluginRegistry::new();
    reg.register(Box::new(DummyPlugin));

    install_project(
        &agents,
        "p1",
        &repo,
        &InstallOptions {
            force: true,
            dry_run: false,
            register_project: true,
            agents: None,
        },
        Some(&reg),
    )
    .unwrap();

    let rule = repo.join(".cursor/rules/global--hello.mdc");
    assert!(rule.is_file());
    assert_eq!(
        fs::metadata(&rule).unwrap().ino(),
        fs::metadata(agents.join("rules/global/hello.md"))
            .unwrap()
            .ino()
    );

    let settings = repo.join(".cursor/settings.json");
    assert!(settings.is_file());

    let cmd = repo.join(".cursor/commands/demo.md");
    assert!(cmd.symlink_metadata().unwrap().file_type().is_symlink());

    let plugin_note = repo.join("PLUGIN_NOTE.txt");
    assert!(plugin_note.symlink_metadata().unwrap().file_type().is_symlink());

    let cfg = agents.join("config.json");
    let raw = fs::read_to_string(&cfg).unwrap();
    assert!(raw.contains("\"p1\""));
}
