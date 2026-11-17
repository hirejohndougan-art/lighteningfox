# Ironswift — français

## Menu contextuel
menu-open-link-new-tab = Ouvrir le lien dans un nouvel onglet
menu-copy-link = Copier l’adresse du lien
menu-open-image-new-tab = Ouvrir l’image dans un nouvel onglet
menu-copy-image = Copier l’adresse de l’image
menu-copy = Copier
menu-back = Précédent
menu-forward = Suivant
menu-reload = Actualiser
menu-select-all = Tout sélectionner
menu-find = Rechercher dans la page…
menu-save-pdf = Enregistrer en PDF…
menu-print = Imprimer…
menu-view-source = Code source de la page
menu-inspect = Inspecter

## Téléchargements
dl-progress = { $pct } % · { $got } sur { $total }
dl-so-far = { $got } pour l’instant
dl-done = Terminé · { $size } · cliquez pour ouvrir
dl-failed = Échec : { $error }
dl-cancelled = Annulé
dl-show-all = Afficher tous les téléchargements
dl-open-folder = Ouvrir le dossier Téléchargements

## Messages
toast-zoom = Zoom { $pct } %  ·  Ctrl+0 pour réinitialiser
toast-printed = { $n ->
    [one] 1 page envoyée à l’imprimante
   *[other] { $n } pages envoyées à l’imprimante
}
toast-pdf-saved = { $file } ({ $n ->
    [one] 1 page
   *[other] { $n } pages
}) enregistré dans Téléchargements
toast-pdf-no-printer = Aucune imprimante n’a répondu ; { $file } ({ $n ->
    [one] 1 page
   *[other] { $n } pages
}) enregistré dans Téléchargements
toast-pdf-failed = Impossible d’enregistrer le PDF : { $error }

## Fenêtre et onglets
window-loading = Chargement de { $target } — Ironswift
private-badge = Privé

## Rechercher dans la page
find-none = Aucun résultat
find-count = { $current } sur { $total }
find-hint = Entrée : suivant  ·  Maj+Entrée : précédent  ·  Échap : fermer

## Pages d’erreur
err-try-again = Réessayer
err-settings = Paramètres
err-details = Détails
err-dns-title = Ce site est introuvable
err-dns-detail = Aucune adresse n’a été trouvée pour { $host }.
err-dns-tip1 = Vérifiez l’orthographe de l’adresse.
err-dns-tip2 = Vérifiez que vous êtes connecté à Internet.
err-dns-tip3 = Le site n’existe peut-être plus.
err-timeout-title = Le site a mis trop de temps à répondre
err-timeout-detail = { $host } n’a pas répondu à temps.
err-timeout-tip1 = Réessayez dans un instant ; le site est peut-être surchargé.
err-net-tip = Vérifiez votre connexion Internet.
err-cert-title = Cette connexion n’est pas sécurisée
err-cert-detail = { $host } a envoyé un certificat de sécurité auquel Ironswift ne peut pas se fier, la page n’a donc pas été chargée. Quelqu’un essaie peut-être de lire ou de modifier ce que vous envoyez.
err-cert-tip1 = S’il s’agit de votre site ou de celui de votre entreprise, son certificat a peut-être expiré ou n’est pas installé.
err-cert-tip2 = Ne saisissez ni mot de passe ni données de paiement sur ce site.
err-refused-title = Le site a refusé la connexion
err-refused-detail = { $host } n’accepte pas de connexions.
err-refused-tip1 = Le site est peut-être en panne ou ne fonctionne pas sur ce port.
err-refused-tip2 = Réessayez plus tard.
err-proxy-title = Impossible de se connecter via le proxy
err-proxy-detail = Le serveur proxy n’a pas laissé passer la requête.
err-proxy-tip = Vérifiez le proxy dans les Paramètres (Réseau).
err-type-title = Ironswift ne peut pas afficher ce fichier
err-type-detail = Ce type de fichier ne peut pas être affiché dans un onglet.
err-other-title = Impossible de charger cette page
err-other-detail = Un problème est survenu lors du chargement de { $host }.
err-other-tip = Réessayez avec ↻.

## Paramètres
set-language = Langue
set-language-auto = Comme le bureau
set-language-hint = Pour les menus et messages d’Ironswift, et la langue indiquée aux sites. Prend pleinement effet au prochain démarrage d’Ironswift.
set-high-contrast = Contraste élevé
set-high-contrast-hint = Affiche les pages et le navigateur en noir, blanc et jaune, sans images d’arrière-plan, pour une lecture plus facile.
