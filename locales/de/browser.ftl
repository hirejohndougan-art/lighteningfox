# Ironswift — Deutsch

## Kontextmenü
menu-open-link-new-tab = Link in neuem Tab öffnen
menu-copy-link = Linkadresse kopieren
menu-open-image-new-tab = Bild in neuem Tab öffnen
menu-copy-image = Bildadresse kopieren
menu-copy = Kopieren
menu-back = Zurück
menu-forward = Vor
menu-reload = Neu laden
menu-select-all = Alles auswählen
menu-find = Auf der Seite suchen…
menu-save-pdf = Als PDF speichern…
menu-print = Drucken…
menu-view-source = Seitenquelltext anzeigen
menu-inspect = Untersuchen

## Downloads
dl-progress = { $pct } % · { $got } von { $total }
dl-so-far = bisher { $got }
dl-done = Fertig · { $size } · zum Öffnen klicken
dl-failed = Fehlgeschlagen: { $error }
dl-cancelled = Abgebrochen
dl-show-all = Alle Downloads anzeigen
dl-open-folder = Download-Ordner öffnen

## Hinweise
toast-zoom = Zoom { $pct } %  ·  Strg+0 setzt zurück
toast-printed = { $n ->
    [one] 1 Seite an den Drucker gesendet
   *[other] { $n } Seiten an den Drucker gesendet
}
toast-pdf-saved = { $file } ({ $n ->
    [one] 1 Seite
   *[other] { $n } Seiten
}) in Downloads gespeichert
toast-pdf-no-printer = Kein Drucker hat geantwortet; { $file } ({ $n ->
    [one] 1 Seite
   *[other] { $n } Seiten
}) in Downloads gespeichert
toast-pdf-failed = Das PDF konnte nicht gespeichert werden: { $error }

## Fenster und Tabs
window-loading = { $target } wird geladen — Ironswift
private-badge = Privat

## Auf der Seite suchen
find-none = Keine Treffer
find-count = { $current } von { $total }
find-hint = Eingabe: weiter  ·  Umschalt+Eingabe: zurück  ·  Esc: schließen

## Fehlerseiten
err-try-again = Erneut versuchen
err-settings = Einstellungen
err-details = Details
err-dns-title = Diese Website wurde nicht gefunden
err-dns-detail = Für { $host } wurde keine Adresse gefunden.
err-dns-tip1 = Prüfen Sie die Schreibweise der Adresse.
err-dns-tip2 = Prüfen Sie, ob Sie mit dem Internet verbunden sind.
err-dns-tip3 = Die Website existiert möglicherweise nicht mehr.
err-timeout-title = Die Website hat zu lange nicht geantwortet
err-timeout-detail = { $host } hat nicht rechtzeitig geantwortet.
err-timeout-tip1 = Versuchen Sie es gleich noch einmal; die Website ist vielleicht ausgelastet.
err-net-tip = Prüfen Sie Ihre Internetverbindung.
err-cert-title = Diese Verbindung ist nicht sicher
err-cert-detail = { $host } hat ein Sicherheitszertifikat gesendet, dem Ironswift nicht vertrauen kann, daher wurde die Seite nicht geladen. Jemand könnte versuchen, mitzulesen oder zu verändern, was Sie senden.
err-cert-tip1 = Wenn dies Ihre eigene Website oder die Ihrer Firma ist, ist ihr Zertifikat vielleicht abgelaufen oder nicht installiert.
err-cert-tip2 = Geben Sie auf dieser Website keine Passwörter oder Zahlungsdaten ein.
err-refused-title = Die Website hat die Verbindung abgelehnt
err-refused-detail = { $host } nimmt keine Verbindungen an.
err-refused-tip1 = Die Website ist vielleicht nicht erreichbar oder läuft nicht auf diesem Port.
err-refused-tip2 = Versuchen Sie es später noch einmal.
err-proxy-title = Keine Verbindung über den Proxy möglich
err-proxy-detail = Der Proxyserver hat die Anfrage nicht durchgelassen.
err-proxy-tip = Prüfen Sie den Proxy in den Einstellungen (Netzwerk).
err-type-title = Ironswift kann diese Datei nicht anzeigen
err-type-detail = Diese Art von Datei kann nicht in einem Tab angezeigt werden.
err-other-title = Diese Seite konnte nicht geladen werden
err-other-detail = Beim Laden von { $host } ist etwas schiefgegangen.
err-other-tip = Versuchen Sie es mit ↻ erneut.

## Einstellungen
set-language = Sprache
set-language-auto = Wie die Arbeitsumgebung
set-language-hint = Für die Menüs und Meldungen von Ironswift und die Sprache, die Websites mitgeteilt wird. Wirkt vollständig nach dem nächsten Start von Ironswift.
set-high-contrast = Hoher Kontrast
set-high-contrast-hint = Zeigt Seiten und den Browser in Schwarz, Weiß und Gelb ohne Hintergrundbilder, damit sie leichter zu lesen sind.

## Screen readers (window, tabs, toolbar) and the settings page
a11y-tabs = Tabs
a11y-toolbar = Navigation
a11y-address = Adress- und Suchleiste
tab-untitled = Neuer Tab
button-back = Zurück
button-forward = Vor
button-reload = Neu laden
settings-title = Einstellungen
settings-sub = Nur auf diesem Computer gespeichert.
settings-saved = Einstellungen gespeichert.
settings-cache-cleared = Zwischengespeicherte Dateien gelöscht ({ $mb } MB frei).
settings-startup = Start und Suche
settings-home = Startseite
settings-search-engine = Suchmaschine
settings-restore-tabs = Die Tabs vom letzten Mal wieder öffnen
settings-bookmarks-bar = Lesezeichenleiste anzeigen
settings-memory = Speicher
settings-sleep = Hintergrund-Tabs schlafen legen nach (Minuten, 0 = nie)
settings-sleep-hint = Ein schlafender Tab gibt seine Seite frei und lädt neu, wenn Sie zurückkehren.
settings-gpu = Mit der Grafikkarte zeichnen
settings-gpu-hint = Flüssigeres Scrollen und weniger Arbeit für den Prozessor; der Grafiktreiber braucht etwa 40–70 MB mehr Speicher. Gilt ab dem nächsten Start von Ironswift.
settings-site-processes = Website-Isolierung: jede Website in einem eigenen Prozess
settings-site-processes-hint = Die Seiten jeder Website laufen in einem eigenen, abgeschotteten Prozess ohne Zugriff auf Ihre Dateien, die Cookies anderer Websites oder direkt aufs Netz. Braucht mehr Speicher. Gilt ab dem nächsten Start von Ironswift.
settings-privacy = Datenschutz
settings-tpc = Cookies mit eingebetteten Seiten teilen
settings-tpc-hint = Standardmäßig aus: eine in andere Seiten eingebettete Seite (ein Werbe- oder Tracking-Netzwerk, ein Videoplayer) bekommt auf jeder Seite einen eigenen Cookie-Speicher und kann Ihnen so nicht folgen.
settings-strip = Tracking-Parameter aus Links entfernen
settings-strip-hint = utm_source, fbclid, gclid und ähnliche.
settings-gpc = Global Privacy Control senden
settings-gpc-hint = Bittet Seiten, Ihre Daten nicht zu verkaufen oder weiterzugeben.
settings-clear-exit = Cookies und Website-Daten beim Schließen von Ironswift löschen
settings-clear-exit-hint = Meldet Sie jedes Mal von Seiten ab; Verlauf und Lesezeichen bleiben erhalten.
settings-cookies-allow = Cookies von diesen Seiten immer erlauben
settings-cookies-allow-hint = Auch wenn sie in andere Seiten eingebettet sind (für Anmelde- oder Kommentarfelder, die sie brauchen).
settings-cookies-block = Cookies von diesen Seiten nie erlauben
settings-cookies-session = Cookies dieser Seiten beim Schließen von Ironswift löschen
settings-partition-hint = Andere in eine Seite eingebettete Seiten bekommen für diese Seite einen eigenen Cookie-Speicher und können Ihnen so nicht von Seite zu Seite folgen.
settings-cookie-sites = Seiten mit Cookies ({ $count })
settings-clear-data = Browserdaten jetzt löschen
settings-clear-data-hint = Verlauf, Cookies, Website-Daten und zwischengespeicherte Dateien. Lesezeichen und Einstellungen bleiben.
settings-private-hint = Für Surfen ohne jede Spur öffnen Sie mit Strg+Umschalt+N ein privates Fenster.
settings-blocking = Blockieren und Sicherheit
settings-block-ads = Werbung blockieren
settings-block-ads-hint = EasyList, uBlock-Filter und die Liste von Peter Lowe: Werbeanfragen werden gestoppt, Adblock-Sperren ausgehebelt und leere Werbeflächen ausgeblendet. Seiten laden schneller.
settings-block-trackers = Tracker blockieren
settings-block-trackers-hint = EasyPrivacy und uBlock Privacy: Statistikdienste, Tracking-Pixel, Fingerprinting-Skripte und Tracking-Parameter in Links.
settings-block-annoyances = Cookie-Hinweise ausblenden
settings-block-annoyances-hint = EasyList Cookie und uBlock Annoyances: Cookie-Banner und Einwilligungsfenster werden ausgeblendet oder geschlossen.
settings-no-blocking = Blockieren auf diesen Seiten abschalten
settings-safe = Vor gefährlichen Seiten warnen
settings-safe-hint = Seiten auf der URLhaus-Schadsoftwareliste und der Phishing-Army-Liste. Die Prüfung erfolgt auf diesem Computer; es wird nichts gesendet.
settings-rfp = Fingerprinting erschweren
settings-rfp-hint = Seiten sehen eine übliche Bildschirmgröße, die Standardschriften und leicht veränderte Canvas-Werte und können Ihren Computer so nicht wiedererkennen.
settings-update-lists = Listen jetzt aktualisieren
settings-permissions-hint = Kamera, Mikrofon, Standort und Benachrichtigungen: Seiten fragen zuerst.
settings-updates = Updates
settings-check-updates = Einmal täglich nach neuen Versionen suchen
settings-check-now = Jetzt suchen
settings-appearance = Darstellung
settings-min-font = Mindestschriftgröße (px, 0 = keine)
settings-min-font-hint = Vergrößert Kleingedrucktes auf Seiten. Strg + und Strg − zoomen eine ganze Seite.
settings-reduce-motion = Bewegung reduzieren
settings-reduce-motion-hint = Bittet Seiten, Animationen gering zu halten.
settings-default-browser = Standardbrowser
settings-make-default = Ironswift zu meinem Standardbrowser machen
settings-network = Netzwerk
settings-proxy = Proxy
settings-proxy-system = Systemeinstellungen
settings-proxy-none = Kein Proxy
settings-proxy-manual = Diesen Proxy verwenden
settings-proxy-address = Proxy-Adresse (für „Diesen Proxy verwenden“)
settings-proxy-hint = „Systemeinstellungen“ folgt dem Proxy des Desktops und den Variablen http_proxy / https_proxy / no_proxy. Sichere Seiten werden mit dem Zertifikatsspeicher des Computers geprüft.
settings-clear-cache = Zwischengespeicherte Dateien löschen
settings-cache-size = ({ $mb } MB gespeichert, damit Seiten schneller laden)
settings-save = Einstellungen speichern
settings-history = Verlauf
settings-bookmarks = Lesezeichen
settings-cookies-deleted = Cookies von { $site } gelöscht.
settings-cookies-allowed = { $site } darf immer Cookies verwenden.
settings-cookies-blocked = { $site } kann keine Cookies mehr verwenden (seine Cookies wurden gelöscht).
settings-cookies-session-set = Die Cookies von { $site } werden beim Schließen von Ironswift gelöscht.
settings-cookies-default = Für { $site } gelten die normalen Cookie-Regeln.
settings-permissions-reset = Berechtigungen für { $site } zurückgesetzt.

## Find bar label
find-label = Suchen

## The content-blocking shield in the toolbar
shield-blocked = { $count } Anfragen auf dieser Seite blockiert
shield-off = Blockieren ist auf { $site } aus
shield-turn-off = Blockieren auf { $site } ausschalten
shield-turn-on = Blockieren auf { $site } wieder einschalten
shield-settings = Einstellungen zum Blockieren…
