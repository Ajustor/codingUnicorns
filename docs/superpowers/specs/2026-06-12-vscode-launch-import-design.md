# Import des configurations de lancement `.vscode`

**Date :** 2026-06-12
**Statut :** Design approuvé

## Objectif

Quand un workspace contient des fichiers `.vscode/launch.json` et/ou
`.vscode/tasks.json`, l'éditeur doit en dériver des options de lancement
prêtes à l'emploi dans le Run panel, et — pour les configurations
débogables — permettre un vrai débogage via le `DapManager` existant.
But : un projet déjà configuré pour VSCode est immédiatement exploitable
sans reconfiguration.

## Décisions produit (validées)

1. **Fichiers pris en charge :** `launch.json` ET `tasks.json`.
2. **Traitement de `launch.json` :** Run terminal + branchement DAP réel
   pour les types débogables.
3. **Fusion :** `launch.toml` + `.vscode` + auto-détection, fusionnés et
   dédupliqués (l'utilisateur voit tout).

## Architecture

Nouveau module **`src/runner/vscode_import.rs`** : fonctions pures, sans I/O
UI, entièrement testables. Il parse les deux fichiers et produit des
`RunConfig` (terminal) et des `DebugSpec` (DAP).

### Composant 1 — Parsing tolérant (JSONC)

Les fichiers `.vscode` contiennent fréquemment des commentaires `//`,
`/* */` et des virgules traînantes que `serde_json` refuse.

```
fn strip_jsonc(input: &str) -> String
```

- Retire les commentaires ligne `//…` et bloc `/* … */`.
- Retire les virgules traînantes avant `}` ou `]`.
- **Conscient des chaînes** : un `//` ou une `,` à l'intérieur d'une chaîne
  JSON (en tenant compte de l'échappement `\"`) ne doit pas être altéré.

En cas d'échec de parsing après nettoyage : le fichier est ignoré
silencieusement (aucun crash ; comportement actuel préservé).

### Composant 2 — `tasks.json` → `RunConfig`

Pour chaque entrée de `tasks[]` :

| Champ VSCode        | Cible `RunConfig`                     |
|---------------------|---------------------------------------|
| `label`             | `name`                                |
| `command`           | `command`                             |
| `args`              | `args`                                |
| `options.cwd`       | `cwd` (défaut `${workspaceRoot}`)     |
| `options.env`       | `env` (paires clé/valeur)             |
| `type` (`shell`/`process`) | informatif, pas de champ dédié |

Traduction des variables VSCode → variables du runner :

| VSCode                          | Runner            |
|---------------------------------|-------------------|
| `${workspaceFolder}`            | `${workspaceRoot}`|
| `${fileDirname}`                | `${fileDir}`      |
| `${fileBasenameNoExtension}`    | `${fileName}`     |
| `${file}`                       | `${file}` (inchangé) |

### Composant 3 — `launch.json` → `RunConfig` + `DebugSpec`

Pour chaque entrée de `configurations[]` :

**a) `RunConfig` (toujours produit)** — commande terminal synthétisée
selon `type` :

| `type`                       | Commande terminal synthétisée        |
|------------------------------|--------------------------------------|
| `python` / `debugpy`         | `python <program> <args>`            |
| `node` / `pwa-node`          | `node <program> <args>`              |
| `go`                         | `go run <program|.> <args>`          |
| `lldb` / `cppdbg` / `coreclr`| exécution directe de `program`       |
| autre                        | exécution directe de `program` si présent, sinon entrée ignorée |

Les variables VSCode du `program`/`args`/`cwd`/`env` sont traduites comme
pour les tâches.

**b) `DebugSpec` (optionnel)** — produit uniquement pour les `type`
débogables connus :

```
struct DebugSpec {
    adapter_cmd: String,
    adapter_args: Vec<String>,
    launch_config: serde_json::Value,
}
```

Table intégrée `type → (adapter_cmd, adapter_args)` :

| `type`                | adapter_cmd | adapter_args                  |
|-----------------------|-------------|-------------------------------|
| `python` / `debugpy`  | `python`    | `["-m", "debugpy.adapter"]`   |
| `node` / `pwa-node`   | `node`      | (adapter js-debug, voir note) |
| `lldb` / `cppdbg`     | `codelldb`  | `["--port", …]` ou stdio      |
| `go`                  | `dlv`       | `["dap"]`                     |
| `coreclr`             | `netcoredbg`| `["--interpreter=vscode"]`    |

Le corps brut de l'entrée `launch.json` (program/args/cwd/env/stopOnEntry…)
devient le `launch_config` passé à `DapConfig`. Type inconnu → pas de
`DebugSpec`, l'entrée reste lançable en terminal uniquement.

> Note `node` : l'adaptateur js-debug de VSCode est complexe et rarement
> disponible en standalone. Si l'adaptateur n'est pas résoluble, le mapping
> DAP est omis (l'entrée reste run-terminal). On ne bloque jamais l'import.

### Composant 4 — Stockage & fusion

`RunConfig` reste **inchangé** : type valeur pur, sérialisation
`launch.toml` propre. Les specs de debug vivent à côté dans `RunManager` :

```
debug_specs: HashMap<String, DebugSpec>   // clé = nom de config, NON sérialisé
```

`debug_specs` est re-dérivé à chaque `load_for_workspace`.

Nouvelle séquence de `load_for_workspace` :

```
let mut configs = vec![];
configs += launch.toml.configurations   // si le fichier existe
configs += import_vscode(workspace)      // tasks + launch → RunConfig
configs += auto_detect_configs(workspace)
dédup(configs)                           // par nom (insensible casse), puis par (command, cwd)
self.configs = configs
self.debug_specs = specs issues de l'import .vscode
```

Dédup : on conserve la première occurrence ; ordre de précédence
`launch.toml` > `.vscode` > auto-détection.

### Composant 5 — Déclenchement du debug (UI)

- **Run panel** (`src/ui/run_panel.rs`) : un bouton **🐞 Debug** à côté de
  ▶, activé seulement si `runner.debug_specs` contient le nom de la config
  active. `RunPanelAction` gagne un champ `debug_clicked: bool`.
- **`start_debug_session`** (`src/app/debug_ops.rs`) : si la config active
  possède une `DebugSpec`, on construit un `DapConfig` à partir d'elle et on
  lance le `DapClient`. Sinon, fallback sur le plugin-par-extension actuel.
  La sémantique F5 existante est préservée.

## Tests

Tests unitaires sur le module pur `vscode_import` (aucune I/O) :

- `strip_jsonc` : commentaires ligne/bloc, virgules traînantes, et un `//`
  ou une `,` à l'intérieur d'une chaîne ne doivent pas être touchés.
- `tasks.json` → `RunConfig` : champs + traduction de variables.
- `launch.json` → `RunConfig` : synthèse de commande par `type`.
- `launch.json` → `DebugSpec` : présence/absence selon `type`, contenu du
  `launch_config`.
- Fusion + dédup : précédence et déduplication par nom puis (command, cwd).

## Hors périmètre (YAGNI)

- Réécriture/sauvegarde des fichiers `.vscode` (lecture seule).
- Variables VSCode avancées (`${command:…}`, `${input:…}`, `${env:…}`).
- `compounds` de `launch.json`.
- Auto-installation des adaptateurs de debug manquants.
