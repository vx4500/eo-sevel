# Travailler à deux (ou plus) sur ce serveur — protocole à suivre

Ce fichier s'adresse à **tout assistant IA** (ChatGPT, Claude, ou autre) qui
intervient sur ce serveur pour modifier `/opt/eo-sevel-src` et déployer sur
`/opt/eo-sevel`. Emmanuel peut demander à plusieurs assistants de travailler
en parallèle sur le même code. Sans coordination, deux agents qui éditent et
déploient en même temps peuvent :

- s'écraser mutuellement des fichiers (le dernier `cp`/écriture gagne, sans
  message d'erreur) ;
- déployer un binaire compilé à partir d'une version du code que l'autre a
  déjà remplacée ;
- se marcher dessus sur le numéro de version (deux agents choisissent le
  même numéro pour deux changements différents) ;
- désinstaller l'outillage (`rustup`/`cargo`) que l'autre est en train
  d'utiliser.

Règle d'or : **avant de toucher au code, annoncez-vous ; avant de compiler ou
déployer, vérifiez que personne d'autre n'est en train de le faire.**

## 1. Fichier de verrou : `.agent-lock.json`

Avant de commencer une session de travail sur `/opt/eo-sevel-src` (édition,
compilation, déploiement), chaque agent doit :

1. Lire `/opt/eo-sevel-src/.agent-lock.json` s'il existe.
2. S'il existe et date de moins de **60 minutes**, un autre agent est
   probablement en train de travailler : **s'arrêter et prévenir
   Emmanuel** plutôt que de continuer.
3. Sinon, créer/écraser ce fichier avec :

```json
{
  "agent": "claude" ou "chatgpt" ou autre nom,
  "debut": "2026-09-19T10:00:00Z",
  "tache": "description courte de ce qui va être fait",
  "pid_session": "identifiant de session si disponible"
}
```

4. Une fois le déploiement terminé et l'outillage nettoyé, **supprimer le
   fichier** (`rm /opt/eo-sevel-src/.agent-lock.json`). Un verrou oublié
   bloquerait l'agent suivant pour rien — s'il en trouve un manifestement
   périmé (plusieurs heures, agent terminé), il peut le supprimer après
   avoir vérifié qu'aucun processus `cargo`/`rustc` ne tourne
   (`pgrep -fa cargo`) et que le service n'est pas en cours de redémarrage.

## 2. Journal partagé : `JOURNAL-AGENTS.md`

Chaque agent ajoute une entrée **à la fin** de
`/opt/eo-sevel-src/JOURNAL-AGENTS.md` après chaque intervention (pas avant,
une fois le travail réellement terminé) :

```
### 2026-09-19 — claude — v2.14.9
Ajout du menu "Afficher" sur l'historique truie, suppression du formulaire
de modification d'effectif sur la fiche bande.
```

Avant de commencer, un agent doit **lire ce fichier** pour savoir ce que
l'autre a fait récemment — c'est la mémoire partagée entre les deux, plus
fiable que de deviner à partir du code seul.

## 3. Un seul agent compile/déploie à la fois

- Vérifier `pgrep -fa cargo` et `systemctl status eo-sevel` avant de lancer
  un build : si une compilation tourne déjà, attendre.
- Ne jamais lancer `rustup self uninstall` si le verrou (`.agent-lock.json`)
  indique qu'un autre agent est en session, même si votre propre tâche est
  terminée.
- Le cycle standard reste : sauvegarde du binaire → arrêt du service →
  copie → `chown eo-sevel:eo-sevel` → `chmod 755` → redémarrage → vérifier
  (`curl` + `md5sum`) → nettoyage de l'outillage **seulement si le verrou
  vous appartient et que vous le retirez ensuite**.

## 4. Version et changelog : un seul numéro à la fois

- Toujours vérifier le numéro de version actuel (`grep version
  Cargo.toml`) juste avant de l'incrémenter — ne pas se fier à un numéro
  mémorisé d'une conversation précédente, l'autre agent a pu déjà avancer.
- Les 5 fichiers doivent rester synchronisés (test
  `version_affichee_et_artifact_sont_synchronises`) : `Cargo.toml`,
  `README.md`, `Etat-projet-eo-suivi-rust.md`, `templates/correctifs.html`,
  `.github/workflows/rust.yml`.

## 5. Avant toute édition : relire l'état réel du fichier

Ne pas se fier à ce qu'un fichier contenait dans une conversation
précédente. Toujours refaire un `Read`/`cat` juste avant d'éditer, car
l'autre agent a pu le modifier entre-temps.

## 6. Recommandation pour Emmanuel

Idéalement, éviter de faire travailler les deux agents **au même moment sur
la même fonctionnalité**. Si vous voulez de la vraie parallélisation sans
risque, le plus sûr serait d'initialiser un dépôt Git local
(`git init` dans `/opt/eo-sevel-src`, sans le pousser nulle part) : chaque
agent pourrait alors travailler sur sa propre branche et vous pourriez
fusionner ou choisir quoi garder, avec un historique clair de qui a fait
quoi. Ce fichier de verrou/journal est une solution plus légère en
attendant, mais Git resterait plus sûr si les interventions se multiplient.
