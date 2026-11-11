-- Forçage minuté de la ventilation d'une salle (connecteur Claude, outil
-- forcer_ventilation, voir routes::mcp) : tant que forcage_fin (UTC,
-- « AAAA-MM-JJ HH:MM:SS ») n'est pas dépassé, eo-regul applique forcage_pct
-- à la place du calcul, puis efface les trois colonnes et reprend la
-- régulation normale tout seul.
--
-- Pour une salle non régulée (actif=0) mais qui a un variateur Zigbee, la
-- ligne ambiance_salle est créée par le connecteur avec la sortie du
-- variateur, et forcage_retour garde le pourcentage à remettre à la fin.
--
-- ALTER TABLE n'est pas rejouable : db.rs n'exécute ce fichier que si la
-- colonne forcage_pct manque encore.
ALTER TABLE ambiance_salle ADD COLUMN forcage_pct REAL;
ALTER TABLE ambiance_salle ADD COLUMN forcage_fin TEXT;
ALTER TABLE ambiance_salle ADD COLUMN forcage_retour REAL;
