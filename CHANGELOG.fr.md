# Journal des modifications

Les changements notables de Coding Unicorns, version par version. Chaque section
est affichée sur la [page de téléchargement](https://ajustor.github.io/codingUnicorns/fr/).
La version anglaise, [`CHANGELOG.md`](CHANGELOG.md), est aussi celle de la fenêtre
de mise à jour de l'application : les deux fichiers évoluent ensemble.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) et les
numéros de version suivent le [versionnage sémantique](https://semver.org/lang/fr/).

## [0.10.6] - 2026-10-09

### Nouveautés

- Bouton **Update all** dans le panneau Extensions (*Installed*), à côté de *Check for updates* : met à jour d'un clic tous les modules qui ont une nouvelle version.
- Nouveau module **Dockerfile** (`Dockerfile`, `Containerfile`, `Dockerfile.*`, `.dockerfile`) : coloration des instructions, options, variables, directives et heredocs, aide au survol de chaque instruction, et docker-langserver pour la validation, la complétion et le formatage.
- Nouveau module **Docker Compose** (`compose.yaml`, `docker-compose.yml` et leurs variantes `*.override.yml`) : coloration YAML avec interpolation `${VAR}`, ancres et blocs, aide au survol des clés Compose, et compose-language-service de Microsoft pour la validation, la complétion et le survol.
- Les modules peuvent reconnaître des fichiers par leur nom plutôt que leur extension (`file_names` dans `manifest.toml`, jokers `*`). Le serveur de langage, la barre d'état et le débogueur en tiennent compte : un `Dockerfile` obtient son serveur de langage.

### Corrections

- Désinstaller un module dont une dépendance npm est scopée (`@scope/paquet`) retire bien ce paquet.

## [0.10.5] - 2026-10-08

### Nouveautés

- Les modules de langage fournissent leur débogueur : une section `[debugger]` du `manifest.toml` indique l'adaptateur, les types de configuration qu'il sert et, au besoin, une archive d'où le télécharger. Les adaptateurs communiquent par stdio ou TCP, et les sessions enfants qu'un adaptateur demande (`startDebugging`) s'ouvrent automatiquement. L'IDE lui-même ne contient aucun débogueur.
- Modules officiels mis à jour, proposés dans le panneau Extensions : **C#** débogue avec netcoredbg, **TypeScript, JavaScript, React, Vue, Svelte et HTML** avec vscode-js-debug (programmes Node, ou pages dans Chrome/Edge), **PowerShell** avec PowerShell Editor Services (tous trois téléchargés au premier lancement), **Python** avec debugpy (installé avec le module), **Go** avec Delve (installé avec le module) et **Rust** avec lldb-dap (fourni par LLVM).
- Le texte qu'un débogueur affiche hors protocole apparaît dans la sortie Debug.
- Nouveau module **JSON** (`.json`, `.jsonc`, `.json5`, `.geojson`, `.webmanifest`) : coloration distinguant clés et valeurs, commentaires et virgules finales, aide au survol des clés de `package.json` et `tsconfig.json`, et vscode-json-language-server pour la validation (y compris `$schema`) et le formatage.
- Les modules peuvent colorer des tokens comme `property`, `operator` et `class`, et transmettre des `initializationOptions` à leur serveur de langage (`lsp_init_options`).

### Modifications

- L'identifiant de langage LSP d'une extension de fichier (par ex. `cs` → `csharp`) vient désormais de son module (`language_ids` dans `manifest.toml`) au lieu d'une table dans l'IDE. **Mettez à jour vos modules** depuis le panneau Extensions : les anciens modules C#, Python et Rust ne le déclarent pas.

### Corrections

- **Le débogueur démarre enfin.** F5 ne faisait rien : aucun module installé ne fournissait d'adaptateur de debug, et l'erreur était silencieuse.
- La `preLaunchTask` d'une configuration de lancement (par ex. `build` de `.vscode/tasks.json`) s'exécute avant le programme, avec sa sortie dans le panneau Debug. Un build en échec arrête le lancement.
- Les problèmes de lancement (débogueur absent, build en échec, erreur de l'adaptateur) s'affichent dans le panneau Debug au lieu d'être perdus.
- Le panneau Debug se met à jour tout seul quand un breakpoint est atteint, sans devoir bouger la souris.
- Une session terminée se relance avec F5 sans passer par Stop ; Stop annule aussi un build en cours.
- Les débogueurs, et les programmes qu'ils exécutent, s'arrêtent quand l'IDE se ferme.

## [0.10.4] - 2026-10-08

### Modifications

- `Ctrl+Shift+F` lance la recherche dans le workspace sur le texte sélectionné (une seule ligne, cherché littéralement en mode regex) et place le focus dans le champ de recherche.
- **Formater le document passe à `Shift+Alt+F`**, comme dans VSCode : `Ctrl+Shift+F` formatait le fichier *et* ouvrait la recherche en même temps.

## [0.10.3] - 2026-10-08

### Corrections

- Gels toutes les deux secondes pendant l'édition ou un build dans un gros dépôt : le rafraîchissement git automatique ajouté en 0.10.2 se fait maintenant en arrière-plan.
- Gel quand la souris restait sur un mot non résolu : la recherche de survol ne lit plus les fichiers du workspace et n'est tentée qu'une fois par mot au lieu de chaque image.
- Le terminal ne se redessine plus en continu quand il est visible, et ne dessine que les lignes à l'écran au lieu de tout son historique.
- Les plugins (compteur de mots de la barre d'état) ne tournent que si le texte, le fichier ou le curseur change, au lieu de copier tout le fichier à chaque image.
- La recherche dans le workspace ne plante plus sur les lignes de résultat contenant des accents.

## [0.10.2] - 2026-10-08

### Nouveautés

- Un clic dans la marge, à gauche des marqueurs de pliage, ajoute ou retire un point d'arrêt (un point pâle le prévisualise au survol) ; aussi dans le panneau de droite en mode split.
- La branche courante, ses compteurs ahead/behind et les fichiers modifiés se mettent à jour d'eux-mêmes après un `git checkout`, `commit`, `pull`… fait hors de l'IDE. Léger : seuls quelques fichiers du dossier git sont comparés, au plus toutes les 2 secondes.

### Corrections

- Les serveurs de langage, adaptateurs de debug, installations d'extensions et la CLI Claude n'ouvrent plus de fenêtre de console sous Windows (par exemple avec l'extension C#).
- « Restart now » après une mise à jour MSI ouvre toujours une nouvelle fenêtre, sur le bon dossier.
- Un workspace relatif enregistré par une ancienne version (`cu .`) est ignoré au lieu d'ouvrir le mauvais dossier (comme le dossier d'installation, avec `cu.cmd`).

## [0.10.1] - 2026-10-08

### Corrections

- `cu` rend la main au terminal tout de suite : l'IDE continue en arrière-plan, détaché du terminal (`--wait` le garde au premier plan sous Linux et macOS).
- `${workspaceFolder}` et `${workspaceRoot}` valent le chemin absolu complet du dossier ouvert, y compris après `cu .`, dans les configurations de lancement et de debug.

## [0.10.0] - 2026-10-08

### Nouveautés

- **Gestion des branches dans le panneau Git**, avec des boutons visibles en plus du clic droit :
  - créer une branche depuis HEAD (même détachée), depuis une autre branche ou depuis un commit du graphe, avec un checkout immédiat en option ;
  - renommer n'importe quelle branche, y compris la branche courante ;
  - supprimer une branche après confirmation, avec un avertissement si ses commits ne sont pas fusionnés ;
  - nom de branche vérifié pendant la saisie, `Entrée` pour valider, `Échap` pour annuler.
- Nouvelle page de téléchargement : thème clair/sombre, fichier adapté à votre système mis en avant, empreintes SHA-256 et historique des versions.

### Corrections

- Les panneaux de la barre latérale ne s'élargissent plus à chaque image.

## [0.9.0] - 2026-10-06

### Nouveautés

- Parcourir et installer les extensions depuis le registre en ligne, directement dans le panneau Extensions.
- Les fichiers s'ouvrent dans la fenêtre déjà lancée au lieu d'en ouvrir une nouvelle.
- Windows : Coding Unicorns est proposé dans « Ouvrir avec » pour les fichiers texte.

### Corrections

- Suivi du nouveau nom du dépôt des modules (`coding-unicorns-modules`) : les configurations existantes sont migrées automatiquement.
- Un plugin enregistré sous un nom déjà utilisé remplace l'ancien au lieu de s'y ajouter.

## [0.8.0] - 2026-10-06

### Nouveautés

- **Éditeur** : retour à la ligne automatique, indentation automatique à `Entrée` et désindentation sur l'accolade fermante.
- **Fichiers** : détection des modifications externes, avec une protection avant d'écraser le fichier ; fins de ligne et BOM préservés, UTF-8 invalide toléré.
- **Session** : onglets ouverts, curseur et défilement restaurés pour chaque workspace ; liste « Open Recent ».
- **Git** : stash (save, apply, pop, drop), bouton Fetch, fusion de l'historique divergent au pull, authentification pour fetch et push.
- **Recherche** : modes regex et mot entier dans la recherche workspace.
- **LSP** : recherche de symboles du fichier et du workspace dans la palette, panneau Problems pour tout le workspace.
- **Terminal** : sélection et copie à la souris, collage entre crochets, `Ctrl+Shift+V` et collage au clic droit, taille du PTY adaptée au panneau.
- **Debug** : expressions surveillées, tous les scopes DAP avec dépliage à la demande.
- **Run** : import de `.vscode/launch.json` en l'absence de `launch.toml`.
- Nouvelle icône licorne, intégrée à l'exécutable Windows ; exécutable et installateur signés.

### Corrections

- Le terminal revient à l'invite quand on tape après avoir remonté l'historique.
- La recherche dans le fichier prend le focus et parcourt chaque occurrence.

## [0.7.2] - 2026-10-05

### Corrections

- L'application redémarre bien après une mise à jour par MSI.

## [0.7.1] - 2026-10-05

### Corrections

- Seule la bibliothèque de la plateforme courante est chargée pour une extension.

## [0.7.0] - 2026-10-05

### Corrections

- **Éditeur** : granularité de l'annulation, aller à la ligne, duplication de ligne, jointure CRLF, suppression multi-curseurs, rechercher-remplacer, surlignage de la recherche.
- **Copier, couper et coller** fonctionnent dans l'éditeur et le terminal.
- **Git** : commits de fusion, ajout de nouveaux dossiers, checkout d'une branche distante ou détachée.
- **Navigation** : `Alt+←` / `Alt+→` parcourent correctement l'historique.
- **LSP et debug** : URI de fichiers valides, `launch` envoyé après l'initialisation DAP.
- Sécurité des extensions : identifiants et archives zip ne peuvent plus sortir de leur dossier.

## [0.6.0] - 2026-10-03

### Nouveautés

- **Mises à jour automatiques** distribuées par GitHub Pages, vérifiées par SHA-256.
- **Claude Code intégré** : panneau de discussion à droite, permissions, compte actif, mode interactif dans un terminal.
- **Interface rafraîchie** : thème clair, couleurs dérivées du thème, notifications éphémères, barre d'état aux couleurs du thème, confirmation avant suppression de fichiers.
- **Markdown** : aperçu natif côte à côte.
- **Éditeur** : ligne courante et guide d'indentation actif surlignés, triple-clic pour sélectionner la ligne, défilement horizontal, complétion unifiée avec surlignage des correspondances.
- Registre central des raccourcis avec détection des conflits.

### Améliorations

- Démarrage non bloquant des serveurs LSP, statut « Loading » pendant leur travail.
- Coloration et minimap beaucoup plus rapides sur les gros fichiers.

## [0.5.0] - 2026-04-10

### Nouveautés

- Améliorations de l'expérience d'édition : minimap, surlignage, défilement et nouveaux alias.

## [0.4.0] - 2026-03-20

### Corrections

- Correction de l'installateur Windows.

## [0.3.0] - 2026-03-19

### Nouveautés

- Premier système de mise à jour de l'application.

## [0.2.0] - 2026-03-18

### Nouveautés

- Débogage via le Debug Adapter Protocol (DAP).

## [0.1.0] - 2026-03-15

- Première version.
