# Journal des modifications

Les changements notables de Coding Unicorns, version par version. Chaque section
est aussi affichée dans la fenêtre de mise à jour de l'application et sur la
[page de téléchargement](https://ajustor.github.io/codingUnicorns/).

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) et les
numéros de version suivent le [versionnage sémantique](https://semver.org/lang/fr/).

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
