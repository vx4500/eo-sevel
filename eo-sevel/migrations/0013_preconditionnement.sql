-- Préconditionnement des salles avant l'arrivée réelle d'une bande
-- (EO_Sevel).
--
-- La régulation existante (0005_ambiance.sql) tire l'âge de la bande du
-- dernier `transfert` réellement enregistré vers la salle : une salle vide
-- n'a pas d'âge, donc pas de consigne — c'est volontaire, on ne devine pas.
-- Mais un préchauffage (IRC en post-sevrage avant l'arrivée des porcelets,
-- par exemple) doit démarrer *avant* ce transfert, pas au moment où il est
-- enregistré. Cette table porte l'information « une bande est attendue ici
-- à telle heure » sans toucher à l'historique des mouvements réels : elle
-- n'est qu'une hypothèse de planification, jamais un fait.
--
-- Une bande par salle à la fois. La ligne devient sans effet dès qu'un vrai
-- transfert existe pour cette salle (`bande_en_place` dans eo-regul le lit
-- en priorité) ou une fois `arrivee_prevue` dépassée de plus de 48h sans
-- arrivée réelle — l'éleveur reste libre de la corriger ou de l'effacer,
-- mais un oubli ne chauffe pas une salle vide indéfiniment.
CREATE TABLE IF NOT EXISTS ambiance_preconditionnement (
    salle_id INTEGER PRIMARY KEY REFERENCES salle(id),
    bande_id INTEGER REFERENCES bande(id),
    arrivee_prevue TEXT NOT NULL,
    heures_avant REAL NOT NULL DEFAULT 8,
    note TEXT
);
