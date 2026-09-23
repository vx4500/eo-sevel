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
