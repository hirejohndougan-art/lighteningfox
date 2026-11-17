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
