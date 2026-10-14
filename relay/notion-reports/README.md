# Relais des signalements → Notion

Petit Cloudflare Worker entre la fenêtre **Help → Report an Issue** de l'IDE et
le tableau Notion **Coding Unicorns — Tickets**. Il garde le jeton Notion côté
serveur, vérifie chaque signalement (type, titre, description, tailles
maximales), limite chaque adresse IP à 5 envois par minute, puis crée un ticket :

- `Statut` = **Backlog** (Claude ne prend que les tickets `Prêt`, donc rien ne
  part en développement sans toi) ;
- `Type` = **Bug** ou **Fonctionnalité** ;
- dans la page : un encadré 🦄 « Reported from the app » avec la version et
  l'OS, la description, et le log de lancement si l'utilisateur l'a joint.

`Version` et `Priorité` restent vides : c'est à toi de les remplir.

## Déploiement (une fois)

1. **Intégration Notion** : sur <https://www.notion.so/profile/integrations>,
   crée une intégration interne « Coding Unicorns reports ». Dans *Capabilities*,
   ne coche que **Insert content** (ni lecture ni modification). Copie le jeton.
2. **Partage du tableau** : ouvre la base *Coding Unicorns — Tickets* →
   `•••` → *Connections* → ajoute l'intégration. Elle ne voit que cette base.
3. **Worker** :
   ```sh
   cd relay/notion-reports
   npm install
   npx wrangler login
   npx wrangler secret put NOTION_TOKEN   # colle le jeton de l'étape 1
   npx wrangler deploy
   ```
   `wrangler deploy` affiche l'URL, du type
   `https://coding-unicorns-reports.<ton-sous-domaine>.workers.dev`.
4. **App** : l'URL à utiliser est cette adresse suivie de `/report`. Mets-la
   dans `DEFAULT_REPORT_URL` (`src/ui/report.rs`) pour qu'elle soit compilée
   dans les prochaines versions. Pour tester avant, ajoute-la dans ta
   `config.toml` : `report_url = "https://…/report"`.

## Tester

```sh
npm test                      # tests unitaires (Node 20+)
curl -X POST https://…/report -H "Content-Type: application/json" \
  -d '{"kind":"bug","title":"Test","description":"Test du relais","version":"dev","os":"test"}'
```

Le second doit répondre `{"ok":true}` et créer un ticket dans le Backlog.

## Logs

`npx wrangler tail` affiche les erreurs renvoyées par Notion (jeton invalide,
base non partagée avec l'intégration…).
