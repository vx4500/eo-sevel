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
