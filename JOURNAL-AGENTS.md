# Journal des interventions des agents IA

Une entrée par intervention, ajoutée **à la fin** une fois le travail
terminé. Voir `COLLABORATION-AGENTS.md` pour le protocole complet.

### 2026-09-19 — claude — mise en place du protocole
Création de `COLLABORATION-AGENTS.md` et de ce journal, suite à une
intervention de ChatGPT en parallèle sur le même serveur. Pas de conflit
constaté à ce jour (aucun fichier modifié en dehors des sessions Claude,
aucun outillage résiduel trouvé). Version en production : 2.14.9.

### 2026-09-19 — codex — équipements EO_Awel, EO_Laezh et EO_Treizh
Ajout demandé par Emmanuel : rubrique Mes équipements après Ma gestion. EO_Awel reprend la ventilation existante ; EO_Laezh et EO_Treizh affichent leur préparation et leurs données prévues sans simuler de connexion automate.

### 2026-09-19 — claude — v2.15.1
Correction du seuil de détection "ventilation inefficace" dans eo-regul :
passait de 60% à 95% de commande, pour éviter une fausse alerte critique
quand le régulateur a encore de la marge (66% commandé, température qui
met plus de 20 min à redescendre par forte chaleur — pas une panne).
Demande d'Emmanuel suite à une alerte réelle en production.

### 2026-09-19 — codex — EO_Treizh et EO_Kren
Renommage complet d’EO_Stasion en EO_Treizh, ajout d’EO_Kren et application du visuel sombre de supervision aux équipements.

### 2026-09-20 — claude — v2.15.3
Trouvé les modèles GTE/GTTT Cooperl demandés par Emmanuel dans Google Drive
(dossier "genetique"/racine du dossier partagé : "GTE 2023 01 31 35TFZ.pdf",
"GTTT 35A7X 3 dernières années .pdf"). Décision avec Emmanuel : refonte
complète du GTTT vers ce format (3 ans + référence nationale) reportée à
une prochaine session (chantier de grande ampleur, ~30 indicateurs). Pour
cette version : ajout d'un filtre de lignes/catégories/fournisseurs sur le
détail des dépenses de bande (masquer ex. un fournisseur de semence type
Yxia/DanBred, total recalculé en direct) et sur le GTE (masquer bloc ou
ligne d'indicateur individuellement, mémorisé par appareil via
localStorage).

### 2026-09-20 — claude — v2.15.4
Refonte du GTE ("elle est pas acceptable" / "reprend comme modèle celle
fournie et remet ta touche") : ajout de periodes_precedentes(),
CoutsPeriode/charger_couts/bloc_couts (volet économique élevage entier :
produit animaux, MSA, alimentation, renouvellement/semence, santé) et
fusionner_periodes() dans gte_periode.rs. Le handler gte() dans mod.rs
calcule maintenant 3 périodes consécutives et les fusionne ; gte.html
affiche une colonne par période + référence IFIP, avec en-tête nom
d'élevage. Les cases à cocher masquer bloc/ligne (2.15.3) conservées.
Décision prise avec Emmanuel : le rapprochement GTTT (ISSF, réformes,
IDSR...) est reporté à une session ultérieure, chantier séparé.

### 2026-09-23 — claude — incident mémoire, règles ajoutées
Une compilation lancée par Claude (cache de 800 Mo copié dans `/tmp`, qui
est en RAM, alors que le swap était déjà plein) a bloqué le serveur de
06:40 à 10:30. Services repartis sans perte de base. À la demande
d'Emmanuel : section 7 de `COLLABORATION-AGENTS.md` — les agents doivent
limiter leur usage de la machine (mémoire, compilations bridées, pas de
fichiers lourds dans `/tmp`, moins d'allers-retours).
Constaté aussi : la version en service (« 2.11.5 », déployée le 21/09 depuis
le portable, hors Git) diverge de la 2.15.5 du dépôt. Ne pas redéployer
depuis Git sans décision d'Emmanuel.

### 2026-09-23 — claude — v2.16.0 (base : dépôt 2.15.5)
Demande d'Emmanuel : « fais avec ce que tu as sur le serveur », rapport
complet de l'élevage au clic sur « Rapport », remettre en ordre la
communication entre services. Les sources de la version déployée le 21/09
(« 2.11.5 », compilée sur le portable, hors Git) sont introuvables : la
2.16.0 repart de la 2.15.5 du dépôt. Repris du 21/09 : bouton Rapport,
saisie rapide « Perte porc après maternité », feuille de style (nav
compacte recentrée, thèmes). **Perdus avec le binaire du 21/09** (à refaire
si Emmanuel le demande) : suivi multi-bandes et séparation bandes truies /
bandes porcs, choix d'apparence (thèmes) dans Paramètres, vue sur plan en
Ambiance, machine à soupe, PDF hebdomadaire serveur (remplacé par la page
imprimable `/rapport`). Binaire du 21/09 conservé :
`/var/backups/eo-sevel/eo-sevel_avant_2.16.0_*`.
Communication eo-sevel/eo-regul : fausse alerte « ventilation inefficace »
supprimée hors pilotage réel ; `eo-regul` passe en TZ=Europe/Paris (drop-in
`claude-2.16.conf`) — les alertes antérieures au 23/09 11h17 sont en UTC.

### 2026-09-23 — claude — v2.17.0 conduite en bandes
Demande d'Emmanuel (liste A–E : planning des 3 bandes, suivi truies/bandes,
places et flux, performances par bande, listes de travail) : « mets tout en
place intelligemment ». Nouveau module `src/routes/conduite.rs` : /conduite,
/conduite/semaine, /flux, /analyse-bandes, alertes à 10 jours dans la cloche
et le Rapport. Places lues dans `parametre.places_*` (26/310/920, réglées le
21/09) avant `reglage.capacite_*` (60/300/800, anciennes, fausses).
Point d'attention : aucune bande n'a d'inventaire et `poids_moyen` n'est jamais
saisi au sevrage — les effectifs de porcs sont estimés (sevrés − morts −
vendus) et l'homogénéité du poids au sevrage n'est pas calculable.
Piège SQLx rencontré : un agrégat NULL (AVG/MIN sans ligne) lu en `f64` rend
0.0, pas une erreur — toujours lire en `Option<T>`.

### 2026-09-23 — claude — v2.18.0 sevrage par bande
Demande d'Emmanuel : une page dans les bandes pour saisir le nombre de
porcelets sevrés, et « une fois le sevrage confirmé, sortir les truies de la
bande ». /bande/{id}/sevrage (src/routes/sevrage.rs) : écart compté → perte,
porcelets répartis dans les cases de post-sevrage, truies hors bande et
déplacées (verraterie proposée).
**Bug de production trouvé et corrigé** : la base réelle (héritée de Python)
a `evenement.suivi_actif` NOT NULL sans valeur par défaut. 17 INSERT de
production l'omettaient (sevrages maternité / saisie rapide / fiche truie,
IA, échos, chaleurs, imports, nourrice) et échouaient en production alors que
les tests passaient. Un test (`chaque_saisie_d_evenement_renseigne_suivi_actif`)
l'impose désormais. Toujours tester une écriture sur une copie de la base
réelle, pas seulement sur la base de test.

### 2026-09-23 — claude — 2.18.2 → retour 2.18.1 → 2.18.3
La 2.18.2 a bloqué la connexion (erreur 415) pendant quelques minutes :
Emmanuel ne pouvait plus se connecter. Retour immédiat au binaire 2.18.1,
puis 2.18.3. Cause : le script global de base.html envoie les formulaires
en multipart (FormData) alors que les routes `Form` attendent de
l'urlencoded ; le 415 était masqué par un renvoi classique. **Ne pas toucher
à ce script sans essai réel dans un navigateur** (Chrome indisponible pour
Claude ce jour-là). Chantier à reprendre : envoyer en urlencoded hors
formulaires de fichiers, pour que les erreurs s'affichent vraiment sur place.

### 2026-09-28 — claude — v2.38.0 caméras des salles
Demande d'Emmanuel : voir les caméras des maternités dans les salles « sans
alourdir ». Les 3 caméras mater (192.168.1.2/.3/.4, serveur `LSWebServer`,
générique type Longse) n'ont **aucune URL de photo** (ISAPI, ONVIF, cgi : 404) ;
seul le RTSP marche (`/Streaming/Channels/102`, ex-config Frigate/HA). D'où
ffmpeg à la demande : ~1,2 s dont 0,16 s CPU par image. Caméra extérieure
Reolink (.104) non ajoutée. Emmanuel saisit lui-même les caméras.
Au passage : test `ameliorations` corrigé (inseminations() prend Query depuis
2.37.0). Reste en échec, sans lien : `zigbee::rattacher_une_sonde_une_lampe_et_un_compteur`
(la base de test n'a pas la table `zigbee_demarrage_reglage` de la migration 0022).
Déploiement : binaire /var/tmp/eo-sevel-2.38.0, installation faite par Emmanuel
(copie du service bloquée pour Claude). Branche `cameras` (worktree
/var/tmp/eo-cameras) à fusionner dans master après le WIP en cours.

### 2026-09-28 — claude — v2.38.1 carte de bande
Remarques d'Emmanuel sur l'accueil (bande 3.40 en verraterie, IA 27/09) :
1re tuile = truies inséminées ; l'« écho confirmée vide 03/09 » venait du
cycle précédent (filtre date >= 1re IA) ; tuile verraterie = truies
actuellement en verraterie (l'ancien compteur ne comptait que les sevrées
transférées). Emplacements triés Engraissement 1 → 5. Les pages /bandes et
/bande gardent l'ancien compteur « vers la verraterie » (flux après sevrage).

### 2026-09-28 — claude (master) — 2.39.3 : ATTENTION aux déploiements depuis la branche `cameras`
La 2.38.1 a été déployée à 07:31 depuis la branche `cameras` (partie de 2.37.2)
alors que master était en 2.39.2 : cela a retiré de la production l'impression
des inséminations, les cases incohérentes, la correction d'effectif après
inventaire (inventairecase.dernier_transfert_id) et les corrections du flux.
Emmanuel l'a vu. La 2.39.3 (master) fusionne `cameras` (2.38.1) et 2.39.2.
**Règle : avant tout déploiement, fusionner master dans sa branche (ou
travailler sur master), et comparer avec le binaire en ligne.**

### 2026-09-28 — claude — 2.40.0 machine à soupe en écoute (branche `soupe`, partie de master 5dad9ef)
Protocole Asserva Exa-Feeder décodé à partir de captures usbmon du câble FTDI
de la VM 101 (Windows XP, Mega-feeder NT) : trames ASCII STX/SOH, adresse,
commande, sous-commande, longueur 4 chiffres, champs « ; », contrôle =
XOR des octets (3 chiffres) + dernier chiffre de la somme (vérifié sur 1500
trames). Champs de l'état `c0` documentés dans src/soupe.rs, validés contre les
écrans d'Exa-Feeder sur un repas complet (28/09 20:17-20:36). Le champ 10 n'est
PAS le n° de passage (compté par EO_Sevel). EO_Sevel n'écrit JAMAIS sur la
machine. Le service `soupe-ecoute` (hôte Proxmox 192.168.1.140) poste vers
EO_Sevel ; Mosquitto n'écoute qu'en local, d'où l'HTTP + jeton.
Emmanuel signalait « Caméras a disparu » : la carte existait, mais le menu
déroulant Mes équipements ne l'avait jamais eue — ajoutée.
Tests : les 10 échecs `dernier_transfert_id` (+ zigbee) préexistent sur master.
Digiclim 6A (La Berru) : ne répond pas au protocole Exa-Feeder sur J19 (A = borne 1),
en attente de la doc Asserva (mail en brouillon).

### 2026-09-29 — claude — 2.40.1 machine à soupe
Écoute en service sur l'hôte Proxmox depuis le 28/09 21:08 (systemd soupe-ecoute,
URL locale 192.168.1.186:8080). Mega-feeder n'envoie `c0` que fenêtre de
supervision ouverte ; la synchro (capture 29/09 12:08) lit R/S/U/V/W/X/Y et « ` »,
désormais stockés bruts dans soupe_donnee (décodage détaillé à faire).

### 2026-09-29 — claude — 2.41.0 ventilation salle vide
Demande d'Emmanuel : salles vides au mini de ventilation (économie), réglable.
Par salle (ventil_vide, NULL = off). La bande « en place » (dernier transfert)
ne suffit pas à dire qu'une salle est occupée : c'est le comptage des animaux
qui décide ; le préconditionnement garde la régulation normale. eo-regul lit
ventil_vide à part (tolère une base non migrée). eo-regul tourne toujours avec
EO_REGUL_SIMULATION=1 : la consigne est calculée/affichée, rien n'est écrit.

### 2026-09-29 — claude — 2.41.1 machine à soupe sans alerte de silence
Demande d'Emmanuel : la machine ne parle que quand elle sert, donc plus
d'alerte « plus aucune nouvelle » (soupe_silence, fermée au démarrage).
Seule l'alerte de sécurité (phase 7, alerte dosage) reste, envoyée par SMS.

### 2026-09-29 — claude — 2.42.0 truies pas venues en chaleur
Saisie groupée des inséminations (bande ou verraterie) : 3e choix « Pas venue
en chaleur » (evenement.type = sans_chaleur, note « Truie N pas venue en
chaleur »). Visible dans l’historique de la truie, celui de la bande et la
colonne Chaleurs de l’état des inséminations. Non compté comme chaleur/retour.

### 2026-09-29 — claude — 2.43.0 page d’accueil au choix
Paramètres → Page d’accueil (parametre accueil_style, pour toute l’exploitation) :
classique (dashboard.html inchangé), cockpit, tournee, flux. src/routes/accueil.rs
enrichit le contexte du tableau de bord (ambiance_etat, farm_alerts, soupe_etat,
frise 18 mois des bandes) ; gabarits accueil_*.html (morceaux partagés par include).
Essai : EO_TEST_BASE=… cargo test -- --ignored accueils_sur_copie (écrit les 4 pages).

### 2026-09-30 — claude — 2.43.2 tour du logiciel
Parcours automatique de 900 pages sur une copie (utilisateur audit, SMS/tunnel
retirés de la copie) : aucune erreur 500/501. Corrigé : Structure « None m² » ;
eo-regul alertes température strictes (> / <) + hystérésis 0,5 °C (Mater 1/2
ouvraient t_basse à 20,0 pour seuil 20,0, 13 fois en 3 jours).

### 2026-09-30 — claude — 2.43.3 cases négatives
Les 22 « cases avec effectif calculé négatif » d’État des données étaient toutes
en maternité (porcelets nés sans transfert d’entrée, sortis au sevrage) : fausse
alerte, le contrôle exclut désormais les maternités comme cases_incoherentes().
Nouvelle page /etat-donnees/cases (inventaire par case, retour sur la page),
liée depuis la ligne du contrôle quand il y a des anomalies.

### 2026-09-30 — claude — 2.44.0 truies improductives
Demande d’Emmanuel : une truie hors bande en cours attend sa remise en bande
(normal) ; au-delà de 2 mois elle est « à déclarer stérile ». Page
/truies/improductives (menu Mes animaux), délai réglable (parametre
improductive_max_jours, 60 par défaut), bouton « Signaler stérile »
(evenement.type = sterile, visible dans l’historique), alerte d’élevage,
État des données compte ces truies au lieu de « Truies actives sans bande ».

### 2026-09-30 — claude — 2.44.2 factures importées à confirmer
Emmanuel ne trouvait pas 2 factures aliment : restées en statut « apercu »
(jamais confirmées). Désormais : envoi de plusieurs PDF → le premier aperçu
s’ouvre, puis chaque confirmation/annulation ouvre le suivant ; bandeau en haut
de /economique avec lien par facture ; alerte d’élevage « Factures importées à
confirmer ». Helper import_en_attente().

### 2026-09-30 — claude — 2.44.3 import facture : plusieurs bandes par ligne
Aperçu d’import (aliment, véto) : « Répartir sur plusieurs bandes » (cases
bandes_ligne_<n>_<id>) → affectationfacturebande manuelle + verrou, bandes=liste,
comme l’affectation manuelle de Pilotage économique. Essayé sur copie (2 bandes).

### 2026-09-30 — claude — 2.45.0 bandes proposées à l’import
Aperçu d’import (aliment, véto, semence, génétique, vente) : bandes présentes à
la date de livraison (affectation::cycle_present, au stade de l’aliment si
reconnu) cochées d’office, autres bandes dans « Autres bandes ». Cases cochées
→ affectation manuelle verrouillée ; vente : une seule bande (plus petit id coché).

### 2026-09-30 — claude — 2.45.1 factures scannées
Facture véto Cooperl scannée 226092028222 : tesseract (mode par défaut) mélangeait
les colonnes → 1 ligne sur 3 et 32178 € au lieu de 32,78 €. OCR désormais
--psm 6 + preserve_interword_spaces, « | » retirés ; code TVA toléré (4 lu « a »)
; avertissement si total des lignes ≠ HORS TAXES. Test sur le texte OCR réel.
Réparé le test ventes (Query ajouté en 2.44.2). 10 échecs préexistants inchangés.

### 2026-09-30 — claude — 2.45.3 résultat par bande : plus-values et retenues
Économie → Résultat par bande : colonnes Plus-values (plus_value €/kg de
l’apport × poids des lots de la bande, déjà dans les recettes) et Retenues
(total_retenues de l’apport, dédoublonné par num_apport, réparti au poids des
lots ; déduit de la marge) + ligne Total (totaux calculés côté Rust).

### 2026-10-01 — claude — 2.46.0 synoptique machine à soupe
Synoptique SVG animé en tête de « En direct » (/equipements/soupe) : phase,
arrivée produit/eau (dosage), cuve remplie au poids (échelle 1500 kg),
mélangeur qui tourne selon %rpm, pompe + circuit animés en distribution,
vanne en cours avec contrôle/consigne. Rafraîchi en place (2.44.1).
Drive « Disk_Exafeeder » (01/10) : www/index.html = web-app Asserva de
Mega-feeder, SOAP POST /soap/IExafeeder sur le PC (port 8080) : GetSynoptique
(EtatAutomate, NumSequence, Numservice, PoidCuve1, Alarme), ListeCycleEffectueGetList,
FicheVanneGetConsigne/GetControle, ConsigneRapideGetList ; écritures
(FicheVannePutConsigne, AcquitterAlarme, EntreeSortieValidation) à NE PAS appeler.

### 2026-10-01 — claude — 2.47.0 pilotage à distance de la machine à soupe
Emmanuel a activé Mega-feeder → Communications externes (port 251, VM 101 =
192.168.1.195, pare-feu XP ouvert). Service SOAP Delphi (WSDL /wsdl/IExafeeder).
src/routes/soupe_pilotage.rs : /equipements/soupe/pilotage (état, distributions
du jour, vannes animaux/% ration), /equipements/soupe/vanne/{n} (fiche complète),
écritures FicheVannePutConsigne / AcquitterAlarme / EntreeSortieValidation avec
case de confirmation obligatoire + db::journal (ancienne → nouvelle valeur).
URL réglable (parametre soupe_soap_url). Écriture demandée explicitement par Emmanuel.

### 2026-10-01 — claude — 2.47.2 plans d’alimentation : tableau « Truies et cochettes »
Demande d’Emmanuel : la catégorie du formulaire d’ajout doit être une liste des
tableaux existants (plus de saisie libre), et cochettes / truies à l’IA /
gestantes / allaitantes sont réunies dans un seul tableau (chaque stade garde
son J0 et sa frise). Nouvelle catégorie `truie_ia` (J0 = sevrage). Les quatre
stades s’affichent même vides. Testé sur copie de la base (port 18231).

### 2026-10-01 — claude — 2.47.3 plans d’alimentation : frise unique des truies
Emmanuel veut « toute la vie alimentaire sur le même tableau, comme les
porcelets » : cochettes → IA → gestation → maternité sont mis bout à bout sur
une seule frise et une seule liste (plans_aliment::fusionner). Chaque stade
garde son J0 ; un stade sans aliment ni durée connue n’apparaît qu’une fois
qu’on y ajoute un aliment.

### 2026-10-01 — claude — 2.47.4 plans d’alimentation : IA et gestation sur le même J0
Précision d’Emmanuel : l’aliment « IA » va de J0 à J20 et l’aliment gestante
de J21 à J115, comptés depuis la même insémination. truie_ia et truie_gestante
partagent donc un seul tronçon de la frise (plus de fausse alerte J0 à J20).

### 2026-10-02 — claude — 2.47.5 import de la redevance DanBred
Facture DanBred « Redevance autorenouvellement » (141.12.pdf) : le pied de page
« Total animaux reproducteurs français livrés… » l’envoyait vers l’import
génétique Cooperl (aperçu « Cochettes / reproducteurs », référence et date
absentes). Elle passe désormais par l’import semence ; lopdf sort une case du
tableau par ligne, d’où `redevance_ht` (dernier nombre après le libellé).
Attention : `cargo test --release` dépasse 2 Go ici (oom-kill) — tests en
profil debug, plafond 3 Go.

### 2026-10-03 — claude — 2.48.0 / 2.48.1 déployées (+ tests remis au vert)
Les 11 tests en échec depuis 2.39.1 venaient des bases de test (0001 seul :
pas de `dernier_transfert_id`, pas de 0022) et d’assertions périmées ; la prod
avait bien colonnes et tables. Une fois le binaire principal vert, cargo a
enchaîné sur tests/presentation.rs, jamais exécuté depuis : Etat-projet,
correctifs.html et rust.yml étaient restés à 2.41.0 / 2.39.3 → resynchronisés.
Demandes d’Emmanuel : frise « Conduite des bandes » agrandie (cockpit, pleine
largeur) et reprise dans Places et flux ; sur /equipements/soupe, « Repas des
7 derniers jours » et « Journal » repliés par défaut. Binaire précédent :
/var/backups/eo-sevel/eo-sevel_avant_2.48.1_20261003_151658 (= 2.47.5).
target/debug/incremental (34 Go) supprimé, disque à 93 %.

## 2026-10-04 20:40 — v2.51.0 DÉPLOYÉE (branche `quai`, worktree /var/tmp/eo-quai, commit cfdb306)
Saisie rapide « Quai abattoir » : porcs sortis des salles (transfert sans destination) mais encore à l'élevage
jusqu'au bon d'apport. Tables quai_porc/quai_affectation (dans migrations/0002_ventelot.sql) ; synchronise_sortie_abattoir
prend d'abord sur le quai. Affichage /departs + Mes salles, « Remettre en case ».
⚠ PAS fusionnée dans master : le WIP soupe non commité (base.html, mod.rs…) bloque le fast-forward.
L'agent soupe doit faire `git merge quai` avant tout déploiement, sinon il efface la 2.51.0.
Sauvegarde : /var/backups/eo-sevel/eo-sevel_avant_2.51.0_20261004_204031 (+ base .db).

## 2026-10-05 05:53 — v2.51.1 DÉPLOYÉE (branche `quai`, commit ba1b6b3)
Page publique /decouvrir (auth.rs public), lien depuis login, pied de page « Version » sans « Rust ».
Toujours PAS fusionnée dans master (WIP soupe non commité). Merger `quai` avant tout déploiement.
Sauvegarde : /var/backups/eo-sevel/eo-sevel_avant_2.51.1_20261005_055326.

## 2026-10-05 06:15 — v2.51.2 DÉPLOYÉE (branche quai) : /decouvrir enrichie, /contact public. Toujours non fusionnée dans master. Sauvegarde eo-sevel_avant_2.51.2_20261005_061520

## 2026-10-05 06:24 — v2.51.3 DÉPLOYÉE (branche quai) : corrections de /decouvrir et /contact. Toujours non fusionnée dans master. Sauvegarde eo-sevel_avant_2.51.3_20261005_062427

## 2026-10-05 13:11 — v2.52.0 DÉPLOYÉE (branche quai) : module Serveur Proxmox (/equipements/proxmox). Non fusionnée dans master. Sauvegarde eo-sevel_avant_2.52.0_20261005_131133

## 2026-10-05 14:10 — v2.52.1 DÉPLOYÉE (branche quai) : salles/ambiance/éclairage accessibles sur téléphone. Non fusionnée dans master.

## 2026-10-05 14:40 — v2.52.2 DÉPLOYÉE (branche quai) + correction données Case 1 La Berru (inventaire id1 dernier_transfert_id=146 ; apport 12 → 109 porcs case 47). Sauvegarde base elevage_avant_case1_berru_20261005_143927.db

## 2026-10-05 15:51 — v2.52.3 DÉPLOYÉE (branche quai) : suppression de salle vide (contrôles détachés). Test navigation adapté au groupe .menu-tel. Non fusionnée dans master.

## 2026-10-06 05:49 — v2.52.4 DÉPLOYÉE (branche quai) : « QR code ».

## 2026-10-06 12:16 — v2.52.5 DÉPLOYÉE (branche quai) : Proxmox https forcé + message 403. Proxmox = https://192.168.1.140:8006 (nœud « proxmox »), jeton root@pam!eosevel sans PVEAuditor pour l'instant.

## 2026-10-08 13:54 — v2.58.7 DÉPLOYÉE (branche assistant, session « Mise en quai porcs abattoir ») : import PDF secours pdftotext + date d'enlèvement à un chiffre. Sauvegarde eo-sevel_avant_2.58.7_20261008_135429

## 2026-10-08 14:02 — v2.58.8 DÉPLOYÉE (branche assistant) : « À faire » de l'accueil cochable. Sauvegarde eo-sevel_avant_2.58.8_20261008_140232

## 2026-10-08 14:35 — v2.59.0 DÉPLOYÉE (branche assistant) : /maternite vue mises-bas en tableau + panneau (modèle A). Sauvegarde eo-sevel_avant_2.59.0_20261008_143459

## 2026-10-08 17:31 — v2.60.2 DÉPLOYÉE (branche assistant) : maternite.html data-indicateur presents sur la ligne. Sauvegarde eo-sevel_avant_2.60.2_20261008_173118

## 2026-10-09 15:06 — v2.64.0 DÉPLOYÉE (branche assistant, commit d8195ea, session « CSuite G2 ») : saisie rapide hors-ligne (file localStorage, bandeau, renvoi auto ; pas de service worker car http) + /analyse-bandes/tableau (colonnes bandes, moyenne, objectif, PDF par impression). Testé en Chromium headless sur copie de base. Sauvegarde eo-sevel_avant_2.64.0_20261009_150650 (+ base).

## 2026-10-09 15:58 — v2.64.1 DÉPLOYÉE (branche assistant) : tableau des bandes + porcs vendus ; apports de réformes (TRUIE VIANDE) marqués reforme=true, exclus des porcs de bande. Données : venteapport 10 et 11 (truies, févr./mai) détachés de B1.52 + transferts supprimés → B1.52=254, B2.07=230, B3.14=66 (conforme aux bons Cooperl). DA915→B1.52 non prouvé par les bons (B1.52 officiel DA952), laissé tel quel. Sauvegarde eo-sevel_avant_2.64.1_20261009_155804 + elevage_avant_2.64.1_reformes_*.db

## 2026-10-10 — v2.64.2 DÉPLOYÉE (branche assistant) : submit global respecte formaction (bug : « Imprimer la sélection » postait vers /truies/affecter-bande et a retiré 18 truies de B1.25 + 0500711 de B2.33 le 10/10 06:29 ; bande_code restauré depuis elevage_avant_2.64.1_reformes_20261009_155804.db, sauvegarde elevage_avant_restauration_B1.25_*). Icône imprimante dans la barre (window.print). Impression d’une bande passée via événements.

## 2026-10-10 — v2.64.3 DÉPLOYÉE (branche assistant) : page /imprimer (bande/truies/toutes, colonnes à cocher, impression auto) ; icône imprimante du haut → /imprimer ; impression.html lance l’impression seule.

## 2026-10-10 — v2.64.4 DÉPLOYÉE (branche assistant) : /imprimer + portées sans_bande et improductives.

## 2026-10-10 — v2.64.5 DÉPLOYÉE (branche assistant) : /imprimer tri au clic, colonne + tri n° vanne.

## 2026-10-10 — v2.64.7 DÉPLOYÉE (branche assistant) : /imprimer/liste tient sur A4 (vérifié par PDF headless), note tronquée à 80 car. ; fiche truie réformée barrée + bandeau rouge. Attention tests : systemctl stop des unités eo-test-* ne tuait pas l’instance → tuer par pgrep -x eo-sevel hors /opt.

## 2026-10-10 — v2.64.8 DÉPLOYÉE (branche assistant) : /archives liste aussi les truies réformées.

## 2026-10-10 12:33 — v2.64.9 DÉPLOYÉE (branche assistant) : sync sortie abattoir ignore les porcs déjà sortis par un Départ confirmé (même bande, J-3..J+1). Données : transferts 209,210 supprimés (doublon 66 porcs B3.14 du 05/10). Sauvegarde elevage_avant_doublon_depart_B3.14_20261010_123323.db. Écarts restants (non expliqués) : B1.05 252 en case vs 279 attendus, B3.14 177 vs 223.
