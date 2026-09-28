#!/usr/bin/env python3
# soupe-ecoute — installé sur l'hôte Proxmox par EO_Sevel (Mes équipements →
# Machine à soupe). Écoute EN LECTURE SEULE le câble USB-série (FTDI 0403:6001)
# que le PC Mega-feeder utilise pour parler à la machine à soupe Asserva, et
# transmet les trames valides à EO_Sevel. N'écrit jamais rien sur le câble :
# il lit une copie du trafic USB fournie par le noyau (usbmon).
import functools
import glob
import struct
import subprocess
import time
import urllib.request

URL = "__URL__/api/soupe/trames"
JETON = "__JETON__"


def ftdi():
    for d in glob.glob("/sys/bus/usb/devices/*"):
        try:
            if open(d + "/idVendor").read().strip() == "0403" and open(d + "/idProduct").read().strip() == "6001":
                return int(open(d + "/busnum").read()), int(open(d + "/devnum").read())
        except OSError:
            pass
    return None


def controle_ok(trame):
    corps = trame[:-5]
    x = functools.reduce(lambda a, v: a ^ v, corps, 0)
    return f"{x:03d}{sum(corps) % 10}".encode() == trame[-5:-1]


attente = []
dernier_envoi = 0.0


def envoyer(force=False):
    global attente, dernier_envoi
    if not attente or (not force and time.time() - dernier_envoi < 2):
        return
    dernier_envoi = time.time()
    corps = "\n".join(attente).encode("utf-8")
    requete = urllib.request.Request(URL, data=corps, method="POST",
                                     headers={"X-Jeton": JETON, "Content-Type": "text/plain; charset=utf-8"})
    try:
        urllib.request.urlopen(requete, timeout=10).read()
        attente = []
    except Exception as e:  # serveur injoignable : on garde les dernières trames
        print("envoi impossible :", e, flush=True)
        attente = attente[-3000:]


def lire(flux, n):
    donnees = b""
    while len(donnees) < n:
        morceau = flux.read(n - len(donnees))
        if not morceau:
            raise EOFError
        donnees += morceau
    return donnees


def ecouter(bus, appareil):
    try:
        subprocess.run(["modprobe", "usbmon"], check=False)
    except OSError:
        pass
    p = subprocess.Popen(["tcpdump", "-i", f"usbmon{bus}", "-U", "-s", "0", "-w", "-"],
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    tampons = {"<": b"", ">": b""}
    try:
        lire(p.stdout, 24)
        while True:
            ts, tu, taille, _ = struct.unpack("<IIII", lire(p.stdout, 16))
            d = lire(p.stdout, taille)
            envoyer()
            if len(d) < 64 or d[9] != 3 or d[11] != appareil or struct.unpack("<H", d[12:14])[0] != bus:
                continue
            type_, point, donnees = chr(d[8]), d[10], d[64:]
            if point == 0x02 and type_ == "S":
                sens, octets = ">", donnees
            elif point == 0x81 and type_ == "C" and len(donnees) > 2:
                # FTDI : 2 octets d'état en tête de chaque paquet de 64.
                sens, octets = "<", b"".join(donnees[i + 2:i + 64] for i in range(0, len(donnees), 64))
            else:
                continue
            for b in octets:
                tampons[sens] += bytes([b])
                if b != 13:
                    continue
                trame, tampons[sens] = tampons[sens], b""
                debut = max(trame.rfind(b"\x01"), trame.rfind(b"\x02"))
                if debut < 0:
                    continue
                trame = trame[debut:]
                if len(trame) < 14 or not controle_ok(trame):
                    continue
                texte = trame[1:-1].decode("latin-1")
                # Les questions d'état répétitives du PC n'apprennent rien.
                if sens == ">" and texte[2:4] == "c0":
                    continue
                attente.append(f"{ts + tu / 1e6:.1f}\t{sens}\t{texte}")
    except EOFError:
        pass
    finally:
        p.kill()
        envoyer(force=True)


while True:
    trouve = ftdi()
    if trouve is None:
        print("câble FTDI 0403:6001 introuvable, nouvel essai dans 30 s", flush=True)
        time.sleep(30)
        continue
    print("écoute du bus", trouve[0], "appareil", trouve[1], flush=True)
    ecouter(*trouve)
    time.sleep(5)
