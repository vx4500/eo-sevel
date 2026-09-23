# Travailler à deux (ou plus) sur ce serveur — protocole à suivre

Ce fichier s'adresse à **tout assistant IA** (ChatGPT, Claude, ou autre) qui
intervient sur ce serveur pour modifier `/opt/eo-sevel-src` et déployer sur
`/opt/eo-sevel`. Emmanuel peut demander à plusieurs assistants de travailler
sur le même code, parfois à des moments proches. Sans coordination, deux
agents qui éditent et déploient en même temps peuvent :

- s'écraser mutuellement des fichiers (le dernier `cp`/écriture gagne, sans
  message d'erreur) ;
- déployer un binaire compilé à partir d'une version du code que l'autre a
  déjà remplacée ;
- se marcher dessus sur le numéro de version (deux agents choisissent le
  même numéro pour deux changements différents) ;
- désinstaller l'outillage (`rustup`/`cargo`) que l'autre est en train
  d'utiliser.

Depuis le 19/09/2026, `/opt/eo-sevel-src` est un **dépôt Git local** (pas de
remote, rien n'est poussé nulle part — c'est uniquement pour se coordonner
sur cette machine). C'est le mécanisme principal. Le fichier de verrou et le
journal (sections 2 et 3) restent une sécurité complémentaire, utile si un
agent ne sait pas utiliser Git ou l'oublie.

## 1. Git : le réflexe avant et après toute intervention

**Avant de commencer, systématiquement :**

```bash
cd /opt/eo-sevel-src
git status
git log --oneline -10
```

- Si `git status` montre des modifications non commitées qui ne sont pas
  les vôtres (vous venez d'arriver), **c'est que l'autre agent est en plein
  travail ou a été interrompu** : ne touchez à rien, prévenez Emmanuel.
- Si le dernier commit n'est pas celui que vous pensiez (numéro de version
  différent de ce dont vous vous souveniez), **faites confiance à Git, pas
  à votre mémoire de conversation** : relisez les fichiers avant d'éditer.

**Après chaque intervention terminée (testée et déployée) :**

```bash
git add -A
git commit -m "vX.Y.Z — résumé court de ce qui a changé"
```

Un commit par version déployée, pas un gros commit fourre-tout. Le message
doit permettre à l'autre agent de comprendre en une ligne ce qui a changé
sans relire tout le diff.

**Si l'autre agent a modifié des fichiers que vous alliez aussi modifier :**
regardez `git diff` / `git show <commit>` pour voir précisément ce qu'il a
fait avant de continuer, plutôt que d'écraser. En cas de doute réel sur la
compatibilité de vos changements avec les siens, préférez vous arrêter et
demander à Emmanuel de trancher plutôt que de deviner.

Le dépôt n'a pas de remote et n'est pas destiné à en avoir un : pas de
`git push`, pas de branche à synchroniser ailleurs. C'est un historique
local, uniquement pour que les deux agents (et Emmanuel) puissent voir qui
a fait quoi et revenir en arrière si besoin (`git revert`, jamais
`git reset --hard` sans que ce soit explicitement demandé).

## 2. Fichier de verrou : `.agent-lock.json`

En complément de Git (qui protège l'historique des fichiers, mais pas la
compilation/le déploiement, qui touchent au serveur en dehors du dépôt) :

Avant de lancer une compilation ou un déploiement, chaque agent doit :

1. Lire `/opt/eo-sevel-src/.agent-lock.json` s'il existe (il est exclu du
   dépôt Git par `.gitignore`, il ne doit jamais être commité).
2. S'il existe et date de moins de **60 minutes**, un autre agent est
   probablement en train de compiler/déployer : **attendre et prévenir
   Emmanuel** plutôt que de lancer un build ou un déploiement en parallèle.
3. Sinon, créer/écraser ce fichier avec :

```json
{
  "agent": "claude" ou "chatgpt" ou autre nom,
  "debut": "2026-09-19T10:00:00Z",
  "tache": "description courte de ce qui va être fait"
}
```

4. Une fois le déploiement terminé et l'outillage nettoyé, **supprimer le
   fichier** (`rm /opt/eo-sevel-src/.agent-lock.json`). Un verrou oublié
   bloquerait l'agent suivant pour rien — s'il en trouve un manifestement
   périmé (plusieurs heures, agent visiblement terminé), il peut le
   supprimer après avoir vérifié qu'aucun processus `cargo`/`rustc` ne
   tourne (`pgrep -fa cargo`) et que le service n'est pas en cours de
   redémarrage.

## 3. Journal partagé : `JOURNAL-AGENTS.md`

En pratique, l'historique Git (section 1) rend ce fichier redondant pour le
détail technique. Il reste utile pour une chose que Git ne montre pas
facilement : le **contexte humain** (pourquoi Emmanuel a demandé ça, ce
qu'il a répondu à une question ambiguë, un point de vigilance pour la
prochaine fois). Ajoutez-y une entrée courte quand ce contexte mérite
d'être gardé, pas pour chaque commit.

## 4. Un seul agent compile/déploie à la fois

- Vérifier `pgrep -fa cargo` et `systemctl status eo-sevel` avant de lancer
  un build : si une compilation tourne déjà, attendre.
- Ne jamais lancer `rustup self uninstall` si le verrou (`.agent-lock.json`)
  indique qu'un autre agent est en session, même si votre propre tâche est
  terminée.
- Le cycle standard reste : `git commit` du code testé → sauvegarde du
  binaire → arrêt du service → copie → `chown eo-sevel:eo-sevel` →
  `chmod 755` → redémarrage → vérifier (`curl` + `md5sum`) → nettoyage de
  l'outillage **seulement si le verrou vous appartient et que vous le
  retirez ensuite**.

## 5. Version et changelog : un seul numéro à la fois

- Toujours vérifier le numéro de version actuel (`grep version
  Cargo.toml`, ou `git log -1`) juste avant de l'incrémenter — ne pas se
  fier à un numéro mémorisé d'une conversation précédente.
- Les 5 fichiers doivent rester synchronisés (test
  `version_affichee_et_artifact_sont_synchronises`) : `Cargo.toml`,
  `README.md`, `Etat-projet-eo-suivi-rust.md`, `templates/correctifs.html`,
  `.github/workflows/rust.yml`.

## 6. Avant toute édition : relire l'état réel du fichier

Ne pas se fier à ce qu'un fichier contenait dans une conversation
précédente. Toujours refaire un `Read`/`cat` juste avant d'éditer — `git
status`/`git log` en tête de séance suffit à détecter qu'un fichier a
changé depuis la dernière fois que vous l'avez vu.

## 7. Ménager la machine : c'est aussi le serveur de production

Ce serveur est une petite VM (2 cœurs, 5 Go de RAM, 5 Go de swap) qui fait
tourner **en même temps** l'application et la régulation de ventilation.
Le 23/09/2026, une compilation lancée par un agent (avec son cache copié
dans `/tmp`, qui est un tmpfs **en mémoire vive**) a saturé RAM et swap :
la machine a été bloquée environ 4 heures, jusqu'au redémarrage.

Règles, pour tout agent :

- **Jamais de gros fichiers dans `/tmp`** (tmpfs = RAM) : pas de copie de
  `target/`, de base, de binaires ou d'archives. Travailler sur le disque
  (`/opt/eo-sevel-src`, `/var/tmp`).
- **Vérifier la mémoire avant de compiler** : `free -h`. Si le swap est
  déjà largement utilisé, ne pas compiler et prévenir Emmanuel.
- **Compiler bridé** : un seul job, priorité basse, plafond mémoire, pour
  que ce soit la compilation qui s'arrête et jamais les services :

```bash
systemd-run --scope -p MemoryMax=2G -p MemorySwapMax=512M \
  nice -n 19 ionice -c3 cargo build --release -j 1
```

- **Pas de compilation en arrière-plan laissée sans surveillance**, ni
  plusieurs compilations/tests en parallèle.
- **Limiter les échanges et les allers-retours** avec le serveur : pas de
  boucles de commandes, de `strings`/copies de binaires de 20 Mo répétées,
  ni de connexions SSH en rafale. Regrouper les vérifications.
- Après intervention, **supprimer ce qu'on a créé** (copies, binaires
  temporaires, sauvegardes intermédiaires devenues inutiles).
