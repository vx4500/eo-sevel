-- Renseigne les plages laissées vides par 0008 sur les bases déjà installées.
--
-- Les identifiants ont été relevés sur l'automate (GET /api/core/io). Les
-- UPDATE ne touchent qu'aux sections encore vides : si vous avez saisi une
-- plage à la main entre-temps, elle est conservée.

UPDATE ipx_section SET id_debut=65552, id_fin=65559,
       note='Les 8 entrées digitales de la carte'
 WHERE nom='Entrées digitales' AND id_debut IS NULL;

UPDATE ipx_section SET id_debut=65560, id_fin=65563,
       note='Les 4 entrées opto-isolées de la carte'
 WHERE nom='Entrées opto-isolées' AND id_debut IS NULL;

UPDATE ipx_section SET id_debut=65568, id_fin=65571, commandable=1,
       note='Les 4 sorties collecteur ouvert — c''est ici que l''on commande'
 WHERE nom='Sorties collecteur ouvert' AND id_debut IS NULL;

-- Sections absentes de 0008, ajoutées seulement si la carte existe déjà.
INSERT INTO ipx_section (nom, source, id_debut, id_fin, commandable, ordre, note)
SELECT 'Retour d''état des relais', 'io', 65544, 65551, 0, 35,
       'Lecture seule : ce que le relais fait réellement'
 WHERE EXISTS (SELECT 1 FROM ipx_section)
   AND NOT EXISTS (SELECT 1 FROM ipx_section WHERE nom='Retour d''état des relais');

INSERT INTO ipx_section (nom, source, id_debut, id_fin, commandable, ordre, note)
SELECT 'Retour d''état des collecteurs ouverts', 'io', 65564, 65567, 0, 65,
       'Lecture seule'
 WHERE EXISTS (SELECT 1 FROM ipx_section)
   AND NOT EXISTS (SELECT 1 FROM ipx_section WHERE nom='Retour d''état des collecteurs ouverts');

INSERT INTO ipx_section (nom, source, id_debut, id_fin, commandable, ordre, note)
SELECT 'Commandes système', 'io', 65572, 65573, 1, 70,
       'Redémarrage de l''automate et bouton de sélection — à manipuler avec précaution'
 WHERE EXISTS (SELECT 1 FROM ipx_section)
   AND NOT EXISTS (SELECT 1 FROM ipx_section WHERE nom='Commandes système');
