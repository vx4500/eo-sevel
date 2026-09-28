-- Caméras IP rattachées aux salles (voir src/routes/cameras.rs). L'image est
-- tirée du flux RTSP à la demande : rien n'est enregistré sur le disque.
CREATE TABLE IF NOT EXISTS camera (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nom TEXT NOT NULL,
    url TEXT NOT NULL,
    identifiant TEXT,
    mot_de_passe TEXT,
    salle_id INTEGER REFERENCES salle(id),
    actif INTEGER NOT NULL DEFAULT 1
);
