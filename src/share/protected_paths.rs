//! Built-in Live Share protected-path policy, mirrored from the Vue reference.
use std::sync::OnceLock;
const PROTECTED_FOLDERS: &[&str] = &[
    ".claude",
    ".codex",
    ".gemini",
    ".cursor",
    ".vscode",
    ".husky",
    ".githooks",
    ".tessel",
    ".opencode",
    ".continue",
    ".windsurf",
    ".kiro",
    ".grok",
    ".agents",
    ".muse",
    ".crush",
    ".roo",
    ".cline",
    ".amazonq",
    ".aider",
    ".goose",
    ".qwen",
    ".kimi",
    ".amp",
    ".junie",
    ".augment",
    ".copilot",
    ".zed",
    ".github",
    ".gitlab",
    ".circleci",
    ".buildkite",
    ".woodpecker",
    ".azure-pipelines",
    ".forgejo",
    ".gitea",
    ".semaphore",
    ".builds",
    ".tekton",
];
const PROTECTED_FILES: &[&str] = &[
    ".mcp.json",
    ".envrc",
    ".gitignore",
    ".gitattributes",
    ".gitmodules",
    ".pre-commit-config.yaml",
    ".pre-commit-hooks.yaml",
    ".gitlab-ci.yml",
    ".gitlab-ci.yaml",
    "jenkinsfile",
    "azure-pipelines.yml",
    "azure-pipelines.yaml",
    "bitbucket-pipelines.yml",
    ".drone.yml",
    ".drone.yaml",
    ".travis.yml",
    "appveyor.yml",
    ".appveyor.yml",
    ".woodpecker.yml",
    ".woodpecker.yaml",
    "cloudbuild.yaml",
    "cloudbuild.yml",
    "buildspec.yml",
    "codemagic.yaml",
    "bitrise.yml",
    ".drone.star",
    ".cirrus.yml",
    ".cirrus.star",
    ".build.yml",
    "buildkite.yml",
    "buildkite.yaml",
    ".buildkite.yml",
    ".buildkite.yaml",
    "cloudbuild.json",
    "codefresh.yml",
    "netlify.toml",
    "vercel.json",
    "claude.md",
    "agents.md",
    "gemini.md",
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    "copilot-instructions.md",
    ".goosehints",
    ".roomodes",
    "qwen.md",
    "grok.md",
    "opencode.json",
    "opencode.jsonc",
    "crush.json",
    ".crush.json",
    ".aider.conf.yml",
    ".aider.conf.yaml",
    ".aider.model.settings.yml",
    ".aider.model.metadata.json",
    ".aiderignore",
    ".cursorignore",
    ".geminiignore",
    ".continuerc.json",
    "continue.config.json",
];
const HOOK_NAMES: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];
/// `relative` and `extra` use normalized project-relative slash-separated paths.
/// Extra folders are supplied by the caller; no Git process or filesystem I/O occurs here.
pub fn is_protected(relative: &str, extra: &[String]) -> bool {
    let lower = relative.to_lowercase();
    let segments: Vec<&str> = if lower.is_empty() {
        Vec::new()
    } else {
        lower.split('/').collect()
    };
    if segments.iter().any(|name| PROTECTED_FOLDERS.contains(name)) {
        return true;
    }
    let name = segments.last().copied().unwrap_or("");
    static PATTERNS: OnceLock<regex::RegexSet> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        regex::RegexSet::new([
            r"^jenkinsfile\..+$",
            r"^\.?lefthook(-local)?\.ya?ml$",
            r"^\.lintstagedrc(\..+)?$",
            r"^lint-staged\.config\.[cm]?[jt]s$",
            r"^\.?commitlint(rc|\.config)(\..+)?$",
        ])
        .expect("static protected-path patterns")
    });
    if !name.is_empty()
        && (PROTECTED_FILES.contains(&name)
            || (patterns.is_match(name)
                && !matches!(
                    name,
                    "jenkinsfile.md" | "jenkinsfile.txt" | "jenkinsfile.rst" | "jenkinsfile.adoc"
                )))
    {
        return true;
    }
    if extra.iter().any(String::is_empty) && segments.len() == 1 && HOOK_NAMES.contains(&name) {
        return true;
    }
    extra.iter().any(|folder| {
        !folder.is_empty()
            && (lower == *folder
                || lower
                    .strip_prefix(folder)
                    .is_some_and(|tail| tail.starts_with('/')))
    })
}
