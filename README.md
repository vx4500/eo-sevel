# EO_Sevel — portage Rust 2.44.2

Cette archive reprend la base EO_Sevel 1.65 sous forme d'un serveur Rust. Elle
n'utilise plus FastAPI, SQLModel, Uvicorn ni Python pour les fonctions déjà
portées. La base reste au format SQLite `elevage.db` afin de conserver la
compatibilité avec les sauvegardes 1.55 à 1.65.

## Ce qui est déjà natif Rust

- serveur HTTP Axum et accès SQLite asynchrone SQLx ;
- création et migration additive des tables historiques, plus les tables
  eau/électricité ;
- connexion, rôles, limitation des tentatives, CSRF sur les formulaires et
  lecture des mots de passe PBKDF2 créés par la version Python ;
- centre de pilotage annuel avec parcours professionnel des huit étapes de
  chaque bande, stade actif, prochaine intervention, délai, dates clés,
  effectif de truies et événements ;
- chaleurs, inséminations groupées, échographies, mises-bas, sevrages,
  mesures ELD/poids/NEC et pertes de porcelets ;
- tableau professionnel de maternité par bande, avec état de chaque truie,
  mise à jour progressive, pertes jusqu'à J+28 et sevrage vers une case/vanne
  de post-sevrage créant le mouvement d'effectif ;
- adoptions entre truies et vers une nourrice artificielle, lots rattachés à leur
  bande, pertes et sevrages tracés sans double comptage ;
- fiche truie organisée en onglets, mesures et historique modifiables, liaison
  case/vanne, soins de portée et analyse de l'IA probablement fécondante ;
- file IA/cochettes après sevrage : cochettes, sorties de maternité et
  retours d'IA sans bande regroupés, affectation groupée, capacité de
  verraterie réglable (31 places par défaut) ;
- compteurs d'eau/électricité, relevés, consommation entre deux index,
  remplacement de compteur, rattachement site/bandes et import CSV ;
- transferts contrôlés des porcs et truies, capacité des cases, inventaires
  remplaçables par date et effectifs recalculés après le dernier comptage ;
- mouvement des porcs par bande ou unité depuis le bouton « + », suivi des
  emplacements, effectif et numéro de marquage ;
- saisie rapide des mises-bas et des pertes de porcs ou porcelets ;
- saisies économiques et résultats par bande (coût/porc, marge, prix net/kg) ;
- import PDF économique Cooperl/Uniporc : aperçu, doublons, sélection
  simultanée de 1 à 5 documents, transaction globale et historique ;
- reclassement automatique des avoirs génétiques et des montants à signe
  moins final ;
- résultats abattoir par bande et saisies sanitaires par morceau ;
- produits, inventaires, commandes modifiables et imprimables, classement des
  ventes par quantité/kg, prix ou chiffre d'affaires, fermeture de la page
  client, feuilles de préparation, sessions et charges de vente directe ;
- utilisateurs, journal, structure, tâches, téléchargement et restauration
  contrôlée de la base ;
- vues de consultation sanitaire, pharmacie, planning et stocks ;
- GTTT centralisé et filtrable par période/bande, productivité annuelle
  réelle, objectifs pondérés modifiables, productivité par bande/cheptel/rang,
  ELD, résultats techniques d'abattage, réformes, cochettes, repères IFIP,
  charcutiers, effectifs et contrôle d'état des données ;
- entretien, saisies abattoir, cahiers des charges et notes quotidiennes en
  lecture/écriture ;
- suivi engraissement, export CSV mise-bas, import CSV truies, fiches
  imprimables et calendrier ICS ;
- parité des 68 anciennes URL historiquement absentes : saisie rapide,
  attente, scanner/QR, PDF serveur, réglages et plans d'aliment, restauration
  contrôlée, rattachement économique, communications Brevo et détail des
  sessions.

Le détail de la correspondance fonctionnelle avec la 1.65 se trouve dans
`Etat-projet-eo-sevel.md`. L'OCR des images reste volontairement un
module externe : les imports PDF texte sont natifs et les scans sans couche
texte sont refusés.

## Démarrage sur Linux

Installer Rust avec <https://rustup.rs>, puis :

```bash
cargo build --release
ELEVAGE_DATA="$PWD/data" ./target/release/eo-sevel
```

Ouvrir ensuite <http://localhost:8080>.

### Serveur neuf

Sur un serveur Debian 13 vierge, en root, depuis les sources :

```bash
./scripts/installer-debian13.sh
```

Il compile les deux binaires, crée le compte système `eo-sevel`, les dossiers
`/opt/eo-sevel` et `/var/lib/eo-sevel`, installe les unités systemd et démarre
le serveur web. Le service `eo-regul` est installé mais **volontairement laissé
à l'arrêt** : mettre en route une ventilation se décide à la main. Le script
refuse de s'exécuter si une base existe déjà.

### Mise à jour

La mise à jour contrôlée se fait en déposant une archive zip des sources
(même arborescence que ce dépôt) dans
`/var/lib/eo-sevel/maj/eo-sevel-nouvelle-version.zip` : `eo-sevel-update.path`
la détecte et lance `/opt/eo-sevel-src/scripts/mettre-a-jour-zip.sh`. Elle
teste et sauvegarde avant l'arrêt, puis restaure l'ancien binaire si le
contrôle de santé échoue. Elle redéploie `eo-regul` et le redémarre seulement
s'il tournait déjà. Déclenchement manuel :
`sudo /opt/eo-sevel-src/scripts/mettre-a-jour-zip.sh /chemin/vers/archive.zip`.

Sur une base neuve, le compte temporaire est `admin` / `admin`. Le logiciel
oblige à choisir immédiatement un mot de passe d'au moins huit caractères.

## Réutiliser une base 1.65

1. arrêter l'ancienne application ;
2. conserver une copie de sauvegarde ;
3. placer `elevage.db` dans le dossier défini par `ELEVAGE_DATA` ;
4. démarrer la version Rust.

Les migrations sont additives : aucune table ni colonne historique n'est
supprimée. Il ne faut toutefois pas faire fonctionner les versions Python et
Rust en même temps sur la même base.

## Démonstration fictive

Le portail `EO_DEMO_PORTAL=1` dispose d'un historique économique d'au moins
cinq ans pour une conduite en sept bandes actives. Les anciennes démos sont
complétées une seule fois, sans réinitialiser les comptes ni remplacer les
bandes déjà renseignées économiquement. Voir `DEMONSTRATION.md` pour le détail
des données et `scripts/mettre-a-jour-demo.sh` pour le conteneur de démonstration.

## Corrections de saisie

Les traitements sur plusieurs jours sont liés à leur sortie de pharmacie :
modifier le produit ou les dates recalcule la quantité, et supprimer une saisie
erronée restitue uniquement la quantité effectivement déduite. L'opération est
atomique. Les alertes de délai d'attente partent de la fin du traitement.

Pour un ancien traitement sans lien de stock ou après un nouvel inventaire,
cocher « Corriger uniquement l'historique » dans la fiche truie et régulariser
manuellement la pharmacie. Cette option laisse le stock et ses mouvements
inchangés. Modifier seulement une observation ne touche jamais au stock.

Un transfert de truies ne peut pas être daté dans le futur ni précéder un
transfert déjà enregistré pour les animaux sélectionnés.

## Docker

```bash
docker compose up --build -d
```

La base est conservée dans le volume `eo_data`.

## Variables

| Variable | Valeur par défaut | Usage |
| --- | --- | --- |
| `ELEVAGE_DATA` | `data` | dossier de `elevage.db` |
| `EO_HOST` | `0.0.0.0` | adresse d'écoute |
| `EO_PORT` | `8080` | port HTTP |
| `EO_SECURE_COOKIES` | `false` | mettre `true` lorsque le site est servi en HTTPS |
| `EO_MDNS` | activé | `0` coupe l'annonce du serveur sur le réseau local |
| `RUST_LOG` | `info` | niveau du journal |
| `EO_VOCAL_WHISPER` | — | chemin du binaire whisper.cpp ; vide = saisie vocale sans transcription |
| `EO_VOCAL_MODELE` | — | chemin du modèle ggml français (`small` suffit) |
| `EO_VOCAL_FFMPEG` | `ffmpeg` | commande de conversion de l'audio du téléphone |
| `EO_VOCAL_THREADS` | cœurs de la machine | fils de calcul de la transcription |

### Saisie vocale

L'application mobile enregistre l'énoncé, le serveur le transcrit et le
comprend, et l'éleveur relit à l'écran avant que quoi que ce soit ne soit
enregistré. Un ordre ressemble à « truie cinq zéro zéro trois deux cinq, deux
porcelets écrasés » : les numéros se dictent chiffre par chiffre, c'est ce qui
résiste le mieux au bruit d'un bâtiment.

Le découpage suit ce parcours :

1. `POST /api/vocal/analyser` — champ `audio` (ou `texte` déjà transcrit).
   Transcrit, reconnaît le numéro, la quantité et la cause, cherche la portée
   en cours. **Aucune écriture dans l'élevage à cette étape.**
2. l'application affiche la relecture : numéro, effectif présent de la portée,
   perte annoncée, texte brut de la transcription, et propose les numéros
   voisins quand celui compris n'existe pas au cheptel ;
3. `POST /api/vocal/valider` — enregistre ce qui a été relu, en passant par les
   mêmes contrôles que la saisie manuelle (portée non sevrée, cause
   configurée, perte inférieure à l'effectif présent, J+28).

`GET /api/vocal/contexte` fournit à l'application le jeton CSRF, les causes de
perte de l'élevage et la longueur des numéros de travail. Les causes ne sont
jamais inventées : seules celles configurées dans l'élevage sont proposées.

La durée de chaque transcription est journalisée : `journalctl -u eo-sevel
| grep "transcription vocale"` dit s'il faut un modèle plus léger. Le modèle
`base` transcrit environ trois fois plus vite que `small`, ce qui suffit
largement pour des chiffres dictés un par un et une dizaine de mots de métier.
Sur une machine à deux cœurs, `small` demande une dizaine de secondes par
dictée : `base` y est nettement plus confortable.

Une amorce de vocabulaire est passée au moteur — truie, porcelets, écrasés,
mise bas — sans quoi « porcelets écrasés » ressort en « pour se les écraser ».
Et comme la transcription d'une suite de chiffres ajoute ou perd volontiers un
chiffre, le numéro entendu est rapproché du cheptel réel : les truies dont le
numéro est à deux chiffres près de ce qui a été dit sont proposées d'un tap,
plutôt que la dictée rejetée.

Sans `EO_VOCAL_WHISPER` et `EO_VOCAL_MODELE`, tout reste utilisable avec un
texte déjà transcrit — par le téléphone, par exemple — et seule la
transcription côté serveur est indisponible.

Les enregistrements audio sont conservés dix jours, le temps de comprendre les
dictées mal transcrites (`GET /api/vocal/journal`, puis
`GET /api/vocal/{id}/audio`), puis effacés automatiquement. Le texte transcrit,
lui, reste.

### Découverte sur le réseau local

Le serveur s'annonce en mDNS/DNS-SD sous le service `_eosuivi._tcp.local.`,
avec son port, sa version et le nom de l'élevage (`nom_elevage`) comme nom
d'instance. Un élevage sans accès internet n'a ni nom de domaine ni DNS : c'est
ce qui permet à l'application mobile de trouver le serveur sans qu'on lui saisisse
une adresse. Pour vérifier depuis un poste Linux du même réseau :

```bash
avahi-browse -rt _eosuivi._tcp
```

L'annonce n'est jamais bloquante : si le réseau filtre le multicast ou si aucune
interface ne convient, le serveur démarre quand même et un avertissement est
journalisé. Mettre `EO_MDNS=0` pour la couper (serveur derrière un proxy, ou
réseau où le multicast n'est pas souhaité).

## Ambiance et régulation de ventilation

Deux binaires sont produits, et la séparation est le cœur du dispositif :

- `eo-sevel` : le serveur web, comme avant.
- `eo-regul` : le démon de régulation. Il ne partage **aucun code** avec le
  serveur. Ils se parlent uniquement par la base : l'application écrit les
  consignes, le démon écrit les mesures et l'état. Un déploiement, une
  migration de schéma ou un plantage de l'application n'arrête donc jamais la
  ventilation.

### Mise en route

1. Créer les salles dans **Structure** si ce n'est pas fait.
2. Dans **Ambiance et ventilation**, renseigner pour chaque salle l'adresse de
   l'automate et les identifiants d'entrées/sorties, puis cocher **Mesurer**.
   Ne pas cocher « Piloter » tout de suite.
3. Régler les gabarits d'accès à l'automate dans la table `parametre` :
   `ambiance_ipx_url_lecture`, `ambiance_ipx_url_ecriture`,
   `ambiance_ipx_champ_valeur`, `ambiance_ipx_champ_ecriture`,
   `ambiance_ipx_cle`. Les jetons `{hote}`, `{id}` et `{cle}` y sont
   remplacés. **Ces gabarits doivent être vérifiés contre le firmware réel de
   l'automate** : les valeurs par défaut sont plausibles, pas garanties.
4. Démarrer `eo-regul` avec `EO_REGUL_SIMULATION=1` (valeur de l'unité
   systemd fournie). Il mesure, enregistre, calcule et alerte, sans jamais
   écrire sur l'automate.
5. Comparer ses consignes avec la régulation en place **sur une bande
   complète**. Quand les deux se superposent, passer `EO_REGUL_SIMULATION=0`
   et cocher « Piloter » sur **une** salle. Puis les autres.

### Le repli de sécurité n'est pas optionnel

Le démon écrit un battement de cœur (`ambiance_heartbeat`) à chaque cycle.
L'automate doit surveiller ce signal de son côté et basculer les salles sur un
pourcentage de sécurité s'il s'arrête. C'est cette règle, et elle seule, qui
rend acceptable de faire tourner la régulation sur une machine virtuelle
susceptible de redémarrer.

Côté logiciel, toute mesure absente, figée ou hors plage de plausibilité fait
passer la salle à `ventil_securite` et ouvre une alerte. Une sonde qui renvoie
0 °C ne doit jamais pouvoir faire couper la ventilation d'une maternité :
c'est le cas couvert par le test `une_sonde_a_zero_declenche_le_repli_et_non_larret`.

### Volume des données

Les mesures sont enregistrées sur variation (pas à intervalle fixe), en
dixièmes d'unité dans un entier. Une tâche horaire replie le brut en moyennes
horaires puis journalières et purge ce qui a été replié. Rétention réglable
par `ambiance_retention_brut_jours` (30) et `ambiance_retention_heure_jours`
(730) ; le journalier n'est jamais purgé. Ordre de grandeur pour treize salles
suivies en continu : une centaine de mégaoctets pour plusieurs années.

### Consigne suivant l'âge de la bande

C'est ce qu'un automate seul ne peut pas faire. La salle occupée et le nombre
de jours de présence se déduisent du dernier transfert enregistré vers cette
salle ; la consigne est interpolée sur la courbe choisie. Avant le premier
point et après le dernier, la valeur extrême est tenue — jamais extrapolée.
Trois courbes de départ sont créées (maternité, post-sevrage, engraissement) ;
elles ne sont insérées que si la table est vide.

## Points de l'automate

Tout ce qui touche à l'automate est regroupé sous **Automate IPX800**
(`/ambiance/automate`), avec une barre d'onglets : Vue d'ensemble (état,
mise en route pas à pas, dernières commandes), Entrées et sorties
(`/ambiance/ipx`), Carte, Éclairage, Explorateur (`/ambiance/ipx/explorateur`)
et Accès (`/ambiance/ipx/acces`).

Ces pages servent à trois choses.

**Régler l'accès** : adresse, clé d'API, gabarits d'appel. Les jetons `{hote}`,
`{id}` et `{cle}` y sont remplacés à chaque requête, ce qui permet de suivre un
changement d'API du firmware sans recompiler. La clé n'est jamais réaffichée et
ne quitte pas le serveur.

**Explorer** : l'automate est interrogé en direct et sa réponse aplatie en
lignes `chemin → valeur`. Un bouton cherche tout seul les chemins qui
répondent. C'est ainsi qu'on relève les identifiants, qui ne se devinent pas.

**Inventorier** : un « point » est une entrée ou une sortie à laquelle on a
donné un nom, un rôle et une unité. La valeur retenue vaut
`brut × coefficient + décalage` — c'est là qu'on écrit qu'une impulsion vaut dix
litres.

### Compteurs d'eau et d'électricité

Un point de rôle compteur doit être rattaché à un compteur de la page « Eau et
électricité ». Le démon `eo-regul` le relève alors **une fois par jour**, à
l'heure choisie, et crée le relevé dans le suivi existant — avec sa répartition
de coût par bande. Un index n'a pas besoin d'être enregistré toutes les trente
secondes : seul l'écart entre deux relevés porte l'information.

Un relevé saisi à la main le même jour l'emporte : le relevé automatique ne
vient jamais écraser ce qui a été constaté sur le compteur lui-même.

### Programmes d'éclairage

Page **Ambiance → Programmes d'éclairage** (`/ambiance/eclairage`). Un
programme commande une ou plusieurs sorties déclarées pilotables, avec trois
conditions combinables : plages horaires (jours, passage de minuit), seuils
de luminosité (allumer sous X lux, éteindre au-dessus de Y, délai de
confirmation, comportement si le capteur ne répond plus) et minuterie
d'extinction (aussi après un allumage manuel). Forçage temporaire depuis la
page, mode essai, pause, duplication, journal.

Déclarer une lampe ne demande aucun chemin d'API : **Ajouter une lampe**
(`/ambiance/eclairage/lampe/nouvelle`) lit les sorties commandables de
l'automate (sections « commandable » de la carte), on essaie « Allumer », on
nomme, on enregistre. Les objets `api/core/io/…` (relais, sorties, bits) se
lisent et se commandent par leur champ `on` (`{"on": true}`), les autres par
les champs réglés dans Accès.

C'est `eo-regul` qui applique les programmes (`eclairage_core.rs`, partagé
avec l'application). Il agit sur les **changements** (début/fin de plage,
tombée du jour, minuterie) et ne réécrit rien entre deux : un geste à la main
tient jusqu'au prochain changement prévu. Le mode essai est propre à chaque
programme, indépendant de `EO_REGUL_SIMULATION` (ventilation).

### Commande manuelle d'une sortie

Une sortie déclarée *pilotable* (avec son chemin d'écriture) peut recevoir une
valeur depuis la page, après confirmation. Deux garde-fous :

- toute commande est journalisée dans `ipx_commande_journal` — qui, quand,
  quelle valeur, quel résultat ;
- la commande est **refusée** sur un point rattaché à une salle en régulation
  automatique. Le démon réécrirait la consigne au cycle suivant, et l'éleveur
  croirait avoir agi alors que rien n'a tenu.

## Entrée de pharmacie par scan

Les boîtes de médicaments vétérinaires portent un **Data Matrix GS1** — un carré
de points, pas un QR code — qui contient le code produit (GTIN), le numéro de
lot et la date de péremption. C'est exactement ce qu'on recopie à la main dans
le registre d'élevage.

Page : **Pharmacie → Entrée par scan**.

Le décodage de l'image est fait par le téléphone (`BarcodeDetector`, natif sur
Chrome Android) ; le serveur ne reçoit que le texte du code et l'interprète. Le
décodeur GS1 est dans `src/gs1.rs`, sans aucune dépendance, avec ses tests —
dont deux cas relevés sur de vraies boîtes de l'élevage.

### Deux points à connaître

**La caméra exige HTTPS.** Les navigateurs n'autorisent l'accès à la caméra
qu'en contexte sécurisé. Depuis `http://<ip>:8080`, le scan ne peut pas
fonctionner — la page l'explique et propose la saisie manuelle du code. Il faut
donc y accéder par l'adresse du tunnel, ce qui va de pair avec
`EO_SECURE_COOKIES=true`.

**Un code ne dit pas quel produit c'est.** Un GTIN est un numéro, et il n'existe
pas de base publique fiable côté vétérinaire. Au premier scan d'un code inconnu,
le logiciel demande le produit et le conditionnement, puis les retient
(`code_produit`). Les boîtes suivantes sont reconnues sans rien saisir.

### Garde-fous

- Rien n'entre en stock sans relecture à l'écran : un scan n'écrit jamais seul.
- Une péremption antérieure à aujourd'hui est refusée.
- Tous les scans sont journalisés (`scan_journal`), y compris ceux qui
  n'aboutissent pas : un code illisible laisse une trace exploitable.
- Une chaîne non interprétable n'est pas une erreur : elle reste utilisable
  comme clé d'association, sans qu'aucun champ ne soit deviné.

## Vérifications

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Pour produire un binaire Linux x86-64 statique, installable sans dépendre de
la version de glibc du serveur :

```bash
sudo apt-get install musl-tools
./scripts/build-linux-musl.sh
```

Le schéma SQLite de l'archive a été exécuté sur une base vierge, puis sur une
copie de la sauvegarde réelle du 16 août 2026. Les tables attendues et
leurs colonnes correspondent ; la base réelle contient en plus une ancienne
table énergie vide, laissée intacte. Les contrôles SQLite `quick_check` et
clés étrangères sont conformes. Chaque mise à jour doit repasser les trois
commandes Cargo ci-dessus sur le serveur avant remplacement du binaire.
