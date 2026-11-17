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

## Screen readers (window, tabs, toolbar) and the settings page
a11y-tabs = Onglets
a11y-toolbar = Navigation
a11y-address = Barre d'adresse et de recherche
tab-untitled = Nouvel onglet
button-back = Précédent
button-forward = Suivant
button-reload = Actualiser
settings-title = Paramètres
settings-sub = Enregistrés sur cet ordinateur uniquement.
settings-saved = Paramètres enregistrés.
settings-cache-cleared = Fichiers en cache effacés ({ $mb } Mo libérés).
settings-startup = Démarrage et recherche
settings-home = Page d'accueil
settings-search-engine = Moteur de recherche
settings-restore-tabs = Rouvrir les onglets de la dernière fois
settings-bookmarks-bar = Afficher la barre des favoris
settings-memory = Mémoire
settings-sleep = Mettre en veille les onglets en arrière-plan après (minutes, 0 = jamais)
settings-sleep-hint = Un onglet en veille libère sa page et se recharge quand vous y revenez.
settings-gpu = Dessiner avec la carte graphique
settings-gpu-hint = Défilement plus fluide et moins de travail pour le processeur ; le pilote graphique utilise environ 40 à 70 Mo de mémoire en plus. Prend effet au prochain démarrage d'Ironswift.
settings-privacy = Vie privée
settings-tpc = Partager les cookies avec les sites intégrés
settings-tpc-hint = Désactivé par défaut : un site intégré à d'autres (une régie publicitaire ou de pistage, un lecteur vidéo) reçoit un stock de cookies séparé sur chaque site, il ne peut donc pas vous suivre.
settings-strip = Retirer les balises de pistage des liens
settings-strip-hint = utm_source, fbclid, gclid et semblables.
settings-gpc = Envoyer Global Privacy Control
settings-gpc-hint = Demande aux sites de ne pas vendre ni partager vos données.
settings-clear-exit = Supprimer les cookies et données de sites à la fermeture d'Ironswift
settings-clear-exit-hint = Vous déconnecte des sites à chaque fois ; l'historique et les favoris sont conservés.
settings-cookies-allow = Toujours autoriser les cookies de ces sites
settings-cookies-allow-hint = Même quand ils sont intégrés à d'autres sites (pour les connexions ou commentaires qui en ont besoin).
settings-cookies-block = Ne jamais autoriser les cookies de ces sites
settings-cookies-session = Supprimer les cookies de ces sites à la fermeture d'Ironswift
settings-partition-hint = Les autres sites intégrés à une page reçoivent leur propre stock de cookies pour cette page, ils ne peuvent donc pas vous suivre d'un site à l'autre.
settings-cookie-sites = Sites avec des cookies ({ $count })
settings-clear-data = Effacer les données de navigation maintenant
settings-clear-data-hint = Historique, cookies, données de sites et fichiers en cache. Les favoris et paramètres restent.
settings-private-hint = Pour naviguer sans rien garder, ouvrez une fenêtre privée avec Ctrl+Maj+N.
settings-blocking = Blocage et sécurité
settings-block-ads = Bloquer les publicités
settings-block-ads-hint = EasyList : les requêtes publicitaires sont stoppées et les cases vides masquées. Les pages se chargent plus vite.
settings-block-trackers = Bloquer les traqueurs
settings-block-trackers-hint = EasyPrivacy : statistiques, pixels de pistage et scripts d'empreinte.
settings-no-blocking = Désactiver le blocage sur ces sites
settings-safe = M'avertir avant les sites dangereux
settings-safe-hint = Sites des listes URLhaus (logiciels malveillants) et Phishing Army. La vérification se fait sur cet ordinateur ; rien n'est envoyé.
settings-rfp = Résister à l'empreinte numérique
settings-rfp-hint = Les sites voient une taille d'écran courante, les polices standard et des lectures de canvas légèrement variées, ils ne peuvent donc pas reconnaître votre ordinateur.
settings-update-lists = Mettre à jour les listes
settings-permissions-hint = Caméra, micro, position et notifications : les sites demandent d'abord.
settings-updates = Mises à jour
settings-check-updates = Rechercher une nouvelle version une fois par jour
settings-check-now = Rechercher maintenant
settings-appearance = Apparence
settings-min-font = Taille de police minimale (px, 0 = aucune)
settings-min-font-hint = Agrandit les petits caractères des pages. Ctrl + et Ctrl − zooment sur tout un site.
settings-reduce-motion = Réduire les animations
settings-reduce-motion-hint = Demande aux sites de limiter les animations au minimum.
settings-default-browser = Navigateur par défaut
settings-make-default = Faire d'Ironswift mon navigateur par défaut
settings-network = Réseau
settings-proxy = Proxy
settings-proxy-system = Paramètres du système
settings-proxy-none = Pas de proxy
settings-proxy-manual = Utiliser ce proxy
settings-proxy-address = Adresse du proxy (pour « Utiliser ce proxy »)
settings-proxy-hint = « Paramètres du système » suit le proxy du bureau et les variables http_proxy / https_proxy / no_proxy. Les sites sécurisés sont vérifiés avec le magasin de certificats de l'ordinateur.
settings-clear-cache = Effacer les fichiers en cache
settings-cache-size = ({ $mb } Mo gardés pour charger les pages plus vite)
settings-save = Enregistrer les paramètres
settings-history = Historique
settings-bookmarks = Favoris
settings-cookies-deleted = Cookies de { $site } supprimés.
settings-cookies-allowed = { $site } peut toujours utiliser des cookies.
settings-cookies-blocked = { $site } ne peut plus utiliser de cookies (ses cookies ont été supprimés).
settings-cookies-session-set = Les cookies de { $site } seront supprimés à la fermeture d'Ironswift.
settings-cookies-default = { $site } suit les règles normales des cookies.
settings-permissions-reset = Autorisations de { $site } réinitialisées.

## Find bar label
find-label = Rechercher
