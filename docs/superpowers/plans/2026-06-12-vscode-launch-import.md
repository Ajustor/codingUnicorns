# Import des configurations de lancement `.vscode` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Note projet (règle utilisateur) :** ne jamais committer sans demande
> explicite. Les étapes « Commit » ci-dessous sont des points de contrôle —
> ne les exécuter que lorsque l'utilisateur le demande.

**Goal:** Dériver des options de lancement (terminal + débogage DAP) à partir des fichiers `.vscode/launch.json` et `.vscode/tasks.json` d'un workspace, et les fusionner avec les configs existantes.

**Architecture:** Un module pur `src/runner/vscode_import.rs` parse les deux fichiers (avec un nettoyeur JSONC maison) et produit des `RunConfig` + des `DebugSpec`. `RunManager::load_for_workspace` fusionne `launch.toml` + `.vscode` + auto-détection (dédupliqués) et stocke les `DebugSpec` dans une map non sérialisée. Le Run panel gagne un bouton Debug ; `start_debug_session` privilégie la `DebugSpec` de la config active avant le fallback plugin-par-extension.

**Tech Stack:** Rust, serde_json, egui / egui-phosphor.

---

## File Structure

- **Create:** `src/runner/vscode_import.rs` — parsing pur `.vscode` → `RunConfig`/`DebugSpec` + nettoyeur JSONC. Tous les tests unitaires y vivent.
- **Modify:** `src/runner/mod.rs` — déclarer le module, ajouter `debug_specs` au `RunManager`, réécrire `load_for_workspace` (fusion + dédup), ajouter `debug_spec_for` + helper `merge_dedup`.
- **Modify:** `src/ui/run_panel.rs` — champ `debug_clicked` sur `RunPanelAction`, bouton Debug conditionnel.
- **Modify:** `src/ui/layout.rs` — câbler `action.debug_clicked` → `start_debug_session`.
- **Modify:** `src/app/debug_ops.rs` — `start_debug_session` privilégie la `DebugSpec` de la config active.

---

## Task 1: Module `vscode_import` + nettoyeur JSONC

**Files:**
- Create: `src/runner/vscode_import.rs`
- Modify: `src/runner/mod.rs` (déclarer le module)

- [ ] **Step 1: Déclarer le module dans `src/runner/mod.rs`**

Ajouter en haut du fichier (après les `use`) :

```rust
pub mod vscode_import;
```

- [ ] **Step 2: Écrire les tests qui échouent (nettoyeur JSONC)**

Créer `src/runner/vscode_import.rs` avec uniquement la signature et les tests :

```rust
//! Import de configurations de lancement depuis les fichiers `.vscode`
//! (`launch.json`, `tasks.json`). Fonctions pures, testables sans I/O.

/// Nettoie le JSONC (commentaires ligne/bloc, virgules traînantes) pour que
/// `serde_json` puisse parser. Conscient des chaînes : le contenu entre `"`
/// (avec échappement `\"`) est préservé verbatim.
pub fn strip_jsonc(input: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_and_block_comments() {
        let src = "{\n  // ligne\n  \"a\": 1, /* bloc */ \"b\": 2\n}";
        let cleaned = strip_jsonc(src);
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn strips_trailing_commas() {
        let src = "{ \"a\": [1, 2,], \"b\": 3, }";
        let cleaned = strip_jsonc(src);
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["a"], serde_json::json!([1, 2]));
        assert_eq!(v["b"], 3);
    }

    #[test]
    fn preserves_slashes_and_commas_inside_strings() {
        let src = "{ \"url\": \"http://x//y\", \"csv\": \"a,b,\" }";
        let cleaned = strip_jsonc(src);
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["url"], "http://x//y");
        assert_eq!(v["csv"], "a,b,");
    }
}
```

- [ ] **Step 3: Lancer les tests pour vérifier l'échec**

Run: `cargo test --lib runner::vscode_import`
Expected: FAIL (`todo!()` panique).

- [ ] **Step 4: Implémenter `strip_jsonc`**

Remplacer le corps `todo!()` :

```rust
pub fn strip_jsonc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push('"');
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next(); // consomme le second '/'
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next(); // consomme '*'
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }

    strip_trailing_commas(&out)
}

/// Retire les virgules suivies (après espaces) d'un `}` ou `]`. String-aware.
fn strip_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;

    for (idx, &c) in chars.iter().enumerate() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c == ',' {
            let mut j = idx + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                continue; // virgule traînante : on la saute
            }
        }
        out.push(c);
    }
    out
}
```

- [ ] **Step 5: Lancer les tests pour vérifier le succès**

Run: `cargo test --lib runner::vscode_import`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/runner/mod.rs src/runner/vscode_import.rs
git commit -m "feat(runner): add JSONC sanitizer for .vscode import"
```

---

## Task 2: Parsing `tasks.json` → `RunConfig`

**Files:**
- Modify: `src/runner/vscode_import.rs`

- [ ] **Step 1: Écrire le test qui échoue**

Ajouter dans le `mod tests` :

```rust
#[test]
fn parses_tasks_with_var_translation() {
    let json: serde_json::Value = serde_json::from_str(
        r#"{ "tasks": [
            { "label": "Build", "type": "shell", "command": "make",
              "args": ["-C", "${workspaceFolder}/sub"],
              "options": { "cwd": "${workspaceFolder}", "env": { "K": "v" } } },
            { "label": "no command" }
        ] }"#,
    ).unwrap();
    let configs = parse_tasks(&json);
    assert_eq!(configs.len(), 1);
    let c = &configs[0];
    assert_eq!(c.name, "Build");
    assert_eq!(c.command, "make");
    assert_eq!(c.args, vec!["-C", "${workspaceRoot}/sub"]);
    assert_eq!(c.cwd, "${workspaceRoot}");
    assert_eq!(c.env, vec![("K".to_string(), "v".to_string())]);
}
```

- [ ] **Step 2: Lancer le test pour vérifier l'échec**

Run: `cargo test --lib runner::vscode_import::tests::parses_tasks_with_var_translation`
Expected: FAIL (`parse_tasks` / import manquants → erreur de compilation).

- [ ] **Step 3: Implémenter `translate_vars` et `parse_tasks`**

Ajouter en haut du fichier les imports et les fonctions (après le doc-comment du module) :

```rust
use crate::runner::RunConfig;
use serde_json::Value;

/// Traduit les variables VSCode vers celles du runner.
fn translate_vars(s: &str) -> String {
    s.replace("${workspaceFolder}", "${workspaceRoot}")
        .replace("${fileDirname}", "${fileDir}")
        .replace("${fileBasenameNoExtension}", "${fileName}")
    // ${file} est identique dans les deux conventions.
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(translate_vars)
                .collect()
        })
        .unwrap_or_default()
}

fn env_pairs(v: Option<&Value>) -> Vec<(String, String)> {
    v.and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, val)| (k.clone(), val.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse la racine d'un `tasks.json` en `RunConfig`.
pub fn parse_tasks(json: &Value) -> Vec<RunConfig> {
    let Some(tasks) = json.get("tasks").and_then(|t| t.as_array()) else {
        return Vec::new();
    };
    let mut configs = Vec::new();
    for task in tasks {
        let Some(label) = task.get("label").and_then(|v| v.as_str()) else {
            continue;
        };
        let command = task.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            continue;
        }
        let options = task.get("options");
        configs.push(RunConfig {
            name: label.to_string(),
            command: translate_vars(command),
            cwd: options
                .and_then(|o| o.get("cwd"))
                .and_then(|v| v.as_str())
                .map(translate_vars)
                .unwrap_or_else(|| "${workspaceRoot}".to_string()),
            env: env_pairs(options.and_then(|o| o.get("env"))),
            args: string_array(task.get("args")),
        });
    }
    configs
}
```

- [ ] **Step 4: Lancer le test pour vérifier le succès**

Run: `cargo test --lib runner::vscode_import`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/runner/vscode_import.rs
git commit -m "feat(runner): parse .vscode/tasks.json into RunConfigs"
```

---

## Task 3: Parsing `launch.json` → `RunConfig` + `DebugSpec`

**Files:**
- Modify: `src/runner/vscode_import.rs`

- [ ] **Step 1: Écrire les tests qui échouent**

Ajouter dans le `mod tests` :

```rust
#[test]
fn launch_python_yields_runconfig_and_debugspec() {
    let json: serde_json::Value = serde_json::from_str(
        r#"{ "configurations": [
            { "name": "Dbg", "type": "python", "request": "launch",
              "program": "${workspaceFolder}/app.py", "args": ["--x"] }
        ] }"#,
    ).unwrap();
    let (configs, specs) = parse_launch(&json);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, "Dbg");
    assert_eq!(configs[0].command, "python ${workspaceRoot}/app.py");
    assert_eq!(configs[0].args, vec!["--x"]);
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].0, "Dbg");
    assert_eq!(specs[0].1.adapter_cmd, "python");
    assert_eq!(specs[0].1.adapter_args, vec!["-m", "debugpy.adapter"]);
}

#[test]
fn launch_unknown_type_with_program_runs_but_no_debugspec() {
    let json: serde_json::Value = serde_json::from_str(
        r#"{ "configurations": [
            { "name": "Exotic", "type": "ruby", "program": "main.rb" }
        ] }"#,
    ).unwrap();
    let (configs, specs) = parse_launch(&json);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].command, "main.rb");
    assert!(specs.is_empty());
}

#[test]
fn launch_unknown_type_without_program_is_skipped() {
    let json: serde_json::Value =
        serde_json::from_str(r#"{ "configurations": [ { "name": "X", "type": "ruby" } ] }"#)
            .unwrap();
    let (configs, specs) = parse_launch(&json);
    assert!(configs.is_empty());
    assert!(specs.is_empty());
}
```

- [ ] **Step 2: Lancer les tests pour vérifier l'échec**

Run: `cargo test --lib runner::vscode_import`
Expected: FAIL (`parse_launch`, `DebugSpec` non définis → erreur de compilation).

- [ ] **Step 3: Implémenter `DebugSpec`, la synthèse de commande, la table d'adaptateurs et `parse_launch`**

Ajouter (au niveau module, sous les helpers existants) :

```rust
/// Spec de débogage dérivée d'une entrée `launch.json` (type débogable connu).
#[derive(Debug, Clone, PartialEq)]
pub struct DebugSpec {
    pub adapter_cmd: String,
    pub adapter_args: Vec<String>,
    pub launch_config: Value,
}

/// Commande terminal synthétisée selon le `type` VSCode.
/// `None` => entrée non lançable (ignorée).
fn synth_command(typ: &str, program: Option<&str>) -> Option<String> {
    match typ {
        "python" | "debugpy" => Some(format!("python {}", program.unwrap_or("${file}"))),
        "node" | "pwa-node" => Some(format!("node {}", program.unwrap_or("${file}"))),
        "go" => Some(format!("go run {}", program.unwrap_or("."))),
        // Types compilés / divers : on exécute directement le programme s'il existe.
        _ => program.map(|p| p.to_string()),
    }
}

/// Adaptateur DAP pour un `type` débogable connu, sinon `None`.
/// Le `type` `node` est volontairement omis (adaptateur js-debug rarement
/// disponible en standalone ; l'entrée reste lançable en terminal).
fn adapter_for_type(typ: &str) -> Option<(&'static str, Vec<&'static str>)> {
    match typ {
        "python" | "debugpy" => Some(("python", vec!["-m", "debugpy.adapter"])),
        "go" => Some(("dlv", vec!["dap"])),
        "lldb" | "cppdbg" => Some(("codelldb", vec![])),
        "coreclr" => Some(("netcoredbg", vec!["--interpreter=vscode"])),
        _ => None,
    }
}

/// Parse la racine d'un `launch.json`. Retourne (RunConfigs terminal,
/// (nom, DebugSpec) pour les types débogables).
pub fn parse_launch(json: &Value) -> (Vec<RunConfig>, Vec<(String, DebugSpec)>) {
    let Some(arr) = json.get("configurations").and_then(|c| c.as_array()) else {
        return (Vec::new(), Vec::new());
    };
    let mut configs = Vec::new();
    let mut specs = Vec::new();
    for cfg in arr {
        let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let typ = cfg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let program = cfg.get("program").and_then(|v| v.as_str()).map(translate_vars);

        let Some(command) = synth_command(typ, program.as_deref()) else {
            continue;
        };
        configs.push(RunConfig {
            name: name.to_string(),
            command,
            cwd: cfg
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(translate_vars)
                .unwrap_or_else(|| "${workspaceRoot}".to_string()),
            env: env_pairs(cfg.get("env")),
            args: string_array(cfg.get("args")),
        });

        if let Some((adapter_cmd, adapter_args)) = adapter_for_type(typ) {
            specs.push((
                name.to_string(),
                DebugSpec {
                    adapter_cmd: adapter_cmd.to_string(),
                    adapter_args: adapter_args.iter().map(|s| s.to_string()).collect(),
                    launch_config: cfg.clone(),
                },
            ));
        }
    }
    (configs, specs)
}
```

- [ ] **Step 4: Lancer les tests pour vérifier le succès**

Run: `cargo test --lib runner::vscode_import`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/runner/vscode_import.rs
git commit -m "feat(runner): parse .vscode/launch.json into RunConfigs and DebugSpecs"
```

---

## Task 4: Fonction d'import I/O `import_vscode`

**Files:**
- Modify: `src/runner/vscode_import.rs`

- [ ] **Step 1: Implémenter la lecture des fichiers (pas de test unitaire — I/O disque)**

Ajouter au niveau module :

```rust
use std::path::Path;

/// Résultat de l'import du répertoire `.vscode` d'un workspace.
#[derive(Debug, Default)]
pub struct VscodeImport {
    pub configs: Vec<RunConfig>,
    /// (nom de config, spec) — clé = `RunConfig.name`.
    pub debug_specs: Vec<(String, DebugSpec)>,
}

/// Lit + nettoie un fichier JSONC. `None` si absent ou illisible/invalide.
fn read_jsonc(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_jsonc(&content)).ok()
}

/// Importe `.vscode/tasks.json` et `.vscode/launch.json` d'un workspace.
/// Tolérant : un fichier absent ou invalide est simplement ignoré.
pub fn import_vscode(workspace: &Path) -> VscodeImport {
    let dir = workspace.join(".vscode");
    let mut result = VscodeImport::default();

    if let Some(json) = read_jsonc(&dir.join("tasks.json")) {
        result.configs.extend(parse_tasks(&json));
    }
    if let Some(json) = read_jsonc(&dir.join("launch.json")) {
        let (configs, specs) = parse_launch(&json);
        result.configs.extend(configs);
        result.debug_specs.extend(specs);
    }
    result
}
```

- [ ] **Step 2: Vérifier la compilation**

Run: `cargo build`
Expected: compile sans erreur (warnings éventuels sur code non encore utilisé — acceptables jusqu'à la Task 5).

- [ ] **Step 3: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/runner/vscode_import.rs
git commit -m "feat(runner): read .vscode dir into RunConfigs and DebugSpecs"
```

---

## Task 5: Fusion dans `RunManager`

**Files:**
- Modify: `src/runner/mod.rs:73-107` (struct `RunManager`, `new`, `load_for_workspace`)

- [ ] **Step 1: Écrire le test qui échoue (dédup de fusion)**

Ajouter un `mod tests` à la fin de `src/runner/mod.rs` :

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_dedup_drops_duplicate_names_and_commands() {
        let configs = vec![
            RunConfig { name: "Run".into(), command: "a".into(), cwd: "w".into(), env: vec![], args: vec![] },
            RunConfig { name: "run".into(), command: "b".into(), cwd: "w".into(), env: vec![], args: vec![] }, // même nom (casse)
            RunConfig { name: "Other".into(), command: "a".into(), cwd: "w".into(), env: vec![], args: vec![] }, // même (command, cwd)
            RunConfig { name: "Keep".into(), command: "c".into(), cwd: "w".into(), env: vec![], args: vec![] },
        ];
        let merged = merge_dedup(configs);
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Run", "Keep"]);
    }
}
```

- [ ] **Step 2: Lancer le test pour vérifier l'échec**

Run: `cargo test --lib runner::tests::merge_dedup_drops_duplicate_names_and_commands`
Expected: FAIL (`merge_dedup` non défini → erreur de compilation).

- [ ] **Step 3: Ajouter les imports + le champ `debug_specs` + `merge_dedup`**

En haut de `src/runner/mod.rs`, ajouter aux `use` :

```rust
use std::collections::{HashMap, HashSet};
use vscode_import::{import_vscode, DebugSpec};
```

Ajouter le champ à la struct `RunManager` (après `workspace`) :

```rust
    /// Specs de debug importées de `.vscode/launch.json`, clé = nom de config.
    /// Non sérialisé : re-dérivé à chaque `load_for_workspace`.
    debug_specs: HashMap<String, DebugSpec>,
```

Dans `RunManager::new`, ajouter au littéral de struct :

```rust
            debug_specs: HashMap::new(),
```

Ajouter le helper de fusion au niveau module (près de `auto_detect_configs`) :

```rust
/// Fusionne en supprimant les doublons : par nom (insensible à la casse),
/// puis par couple (command, cwd). Conserve la première occurrence.
fn merge_dedup(configs: Vec<RunConfig>) -> Vec<RunConfig> {
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut seen_cmds: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    for c in configs {
        let name_key = c.name.to_lowercase();
        let cmd_key = (c.command.clone(), c.cwd.clone());
        if seen_names.contains(&name_key) || seen_cmds.contains(&cmd_key) {
            continue;
        }
        seen_names.insert(name_key);
        seen_cmds.insert(cmd_key);
        out.push(c);
    }
    out
}
```

- [ ] **Step 4: Lancer le test pour vérifier le succès**

Run: `cargo test --lib runner::tests::merge_dedup_drops_duplicate_names_and_commands`
Expected: PASS.

- [ ] **Step 5: Réécrire `load_for_workspace` + ajouter `debug_spec_for`**

Remplacer entièrement le corps de `load_for_workspace` (actuellement `src/runner/mod.rs:91-107`) par :

```rust
    /// Call when the workspace changes. Fusionne `launch.toml`, l'import
    /// `.vscode` et l'auto-détection, dédupliqués.
    pub fn load_for_workspace(&mut self, workspace: &Path) {
        self.workspace = Some(workspace.to_path_buf());
        self.debug_specs.clear();

        let mut configs: Vec<RunConfig> = Vec::new();

        // 1. launch.toml (précédence la plus haute).
        let launch_file = workspace.join(".coding-unicorns").join("launch.toml");
        if launch_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&launch_file) {
                if let Ok(lf) = toml::from_str::<LaunchFile>(&content) {
                    configs.extend(lf.configurations);
                }
            }
        }

        // 2. Import .vscode (tasks.json + launch.json).
        let imported = import_vscode(workspace);
        configs.extend(imported.configs);
        for (name, spec) in imported.debug_specs {
            self.debug_specs.insert(name, spec);
        }

        // 3. Auto-détection (précédence la plus basse).
        configs.extend(auto_detect_configs(workspace));

        self.configs = merge_dedup(configs);
        self.active_config = 0;
    }

    /// Spec de debug pour une config nommée, si importée depuis `.vscode`.
    pub fn debug_spec_for(&self, name: &str) -> Option<&DebugSpec> {
        self.debug_specs.get(name)
    }
```

- [ ] **Step 6: Vérifier compilation + tests**

Run: `cargo test --lib runner`
Expected: PASS (tous les tests runner, dont vscode_import).

- [ ] **Step 7: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/runner/mod.rs
git commit -m "feat(runner): merge .vscode configs into RunManager with dedup"
```

---

## Task 6: Bouton Debug dans le Run panel

**Files:**
- Modify: `src/ui/run_panel.rs:4-7` (`RunPanelAction`) et la fonction `show`

- [ ] **Step 1: Ajouter le champ `debug_clicked` à `RunPanelAction`**

Remplacer la struct (`src/ui/run_panel.rs:4-7`) par :

```rust
pub struct RunPanelAction {
    pub run_clicked: bool,
    pub stop_clicked: bool,
    pub debug_clicked: bool,
}
```

- [ ] **Step 2: Déclarer la variable et l'inclure dans la valeur de retour**

Dans `show`, après `let mut stop_clicked = false;` (≈ ligne 38) ajouter :

```rust
        let mut debug_clicked = false;
```

Et dans le `RunPanelAction { … }` final (≈ ligne 249), ajouter le champ :

```rust
        RunPanelAction {
            run_clicked,
            stop_clicked,
            debug_clicked,
        }
```

- [ ] **Step 3: Ajouter le bouton Debug dans le header**

Dans le `ui.horizontal(|ui| { … })` du header, juste après le bloc du bouton Run/Stop (après sa fermeture `}` ≈ ligne 77, avant la fin du `horizontal`), insérer :

```rust
            let active_debuggable = runner
                .active_config()
                .map(|c| runner.debug_spec_for(&c.name).is_some())
                .unwrap_or(false);
            if active_debuggable
                && ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(egui_phosphor::regular::BUG)
                                .size(16.0)
                                .color(egui::Color32::from_rgb(220, 160, 80)),
                        )
                        .min_size(egui::vec2(32.0, 28.0)),
                    )
                    .on_hover_text("Debug active configuration")
                    .clicked()
            {
                debug_clicked = true;
            }
```

- [ ] **Step 4: Vérifier la compilation**

Run: `cargo build`
Expected: erreur dans `src/ui/layout.rs` (`RunPanelAction` initialisé/destructuré sans `debug_clicked`) — corrigée en Task 7. La compilation de `run_panel.rs` lui-même doit être correcte.

- [ ] **Step 5: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/ui/run_panel.rs
git commit -m "feat(ui): add Debug button to run panel for debuggable configs"
```

---

## Task 7: Câblage `start_debug_session` + layout

**Files:**
- Modify: `src/ui/layout.rs:654-662` (gestion de `RunPanelAction`)
- Modify: `src/app/debug_ops.rs:6-31` (`start_debug_session`)

- [ ] **Step 1: Câbler `action.debug_clicked` dans `layout.rs`**

Dans `SidebarTab::Run` (après le bloc `if action.run_clicked { … }`, ≈ ligne 656), ajouter :

```rust
                        if action.debug_clicked {
                            app.start_debug_session();
                        }
```

- [ ] **Step 2: Réécrire `start_debug_session` pour privilégier la `DebugSpec`**

Remplacer entièrement le corps de `start_debug_session` (`src/app/debug_ops.rs:6-31`) par :

```rust
    /// Démarre une session DAP. Privilégie la `DebugSpec` de la config de run
    /// active (importée de `.vscode/launch.json`) ; sinon, retombe sur le
    /// plugin associé à l'extension du fichier courant.
    pub fn start_debug_session(&mut self) {
        let current_file = self.editor.current_path.clone();
        let workspace = self.workspace_path.clone().unwrap_or_else(|| {
            current_file
                .as_ref()
                .and_then(|p| p.parent().map(|x| x.to_path_buf()))
                .unwrap_or_default()
        });

        // 1. DebugSpec de la config active, le cas échéant.
        let cfg = self
            .runner
            .active_config()
            .and_then(|c| self.runner.debug_spec_for(&c.name))
            .map(|spec| crate::dap::types::DapConfig {
                adapter_cmd: spec.adapter_cmd.clone(),
                adapter_args: spec.adapter_args.clone(),
                launch_config: spec.launch_config.clone(),
            });

        // 2. Fallback : plugin par extension du fichier courant.
        let cfg = cfg.or_else(|| {
            let ext = current_file
                .as_ref()
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .unwrap_or("");
            self.plugin_manager.dap_config_for_ext(ext)
        });

        let Some(cfg) = cfg else {
            return;
        };

        if let Err(e) = self.dap.start_session(&cfg, &workspace, current_file.as_deref()) {
            self.show_terminal = true;
            if let Some(term) = self.terminals.get_mut(self.active_terminal) {
                term.send_input(&format!("echo 'DAP error: {e}'\n"));
            }
        }
        self.show_sidebar = true;
        self.sidebar_tab = SidebarTab::Debug;
    }
```

- [ ] **Step 3: Vérifier la compilation et la suite de tests**

Run: `cargo build`
Expected: compile sans erreur.

Run: `cargo test --lib`
Expected: PASS.

- [ ] **Step 4: Vérification clippy (le projet compile avec `-D warnings`)**

Run: `cargo clippy --all-targets`
Expected: aucune erreur.

- [ ] **Step 5: Commit** *(seulement si l'utilisateur le demande)*

```bash
git add src/ui/layout.rs src/app/debug_ops.rs
git commit -m "feat(debug): launch .vscode debug configs via DAP from run panel"
```

---

## Self-Review (effectuée)

- **Couverture spec :** Composant 1 (JSONC) → Task 1 ; Composant 2 (tasks) → Task 2 ; Composant 3 (launch → RunConfig + DebugSpec) → Task 3 ; I/O d'import → Task 4 ; Composant 4 (stockage + fusion/dédup) → Task 5 ; Composant 5 (UI Debug + déclenchement) → Tasks 6-7. Tests → présents dans Tasks 1-3 et 5.
- **Cohérence des types :** `DebugSpec { adapter_cmd, adapter_args, launch_config }` défini en Task 3, importé en Task 5, consommé en Task 7 (mappé vers `dap::types::DapConfig`). `RunConfig` (champs `name, command, cwd, env, args`) conforme à `src/runner/mod.rs`. `RunPanelAction` étendu en Task 6, destructuré en Task 7.
- **Pas de placeholder :** chaque étape de code contient le code complet.

## Notes d'exécution

- `${file}` n'est pas traduit (identique dans les deux conventions). Les variables VSCode non couvertes (`${command:…}`, `${input:…}`, `${env:…}`) restent telles quelles dans la commande — hors périmètre.
- Le type `node` produit un `RunConfig` terminal mais **pas** de `DebugSpec` (adaptateur js-debug non géré). C'est intentionnel (voir spec).
- Les adaptateurs (`debugpy`, `dlv`, `codelldb`, `netcoredbg`) doivent être présents sur le PATH pour que le debug démarre ; sinon `start_session` échoue proprement et affiche l'erreur dans le terminal.
