# Ironswift — español

## Menú contextual
menu-open-link-new-tab = Abrir enlace en una pestaña nueva
menu-copy-link = Copiar dirección del enlace
menu-open-image-new-tab = Abrir imagen en una pestaña nueva
menu-copy-image = Copiar dirección de la imagen
menu-copy = Copiar
menu-back = Atrás
menu-forward = Adelante
menu-reload = Volver a cargar
menu-select-all = Seleccionar todo
menu-find = Buscar en la página…
menu-save-pdf = Guardar como PDF…
menu-print = Imprimir…
menu-view-source = Ver código fuente
menu-inspect = Inspeccionar

## Descargas
dl-progress = { $pct } % · { $got } de { $total }
dl-so-far = { $got } hasta ahora
dl-done = Terminado · { $size } · haz clic para abrir
dl-failed = Error: { $error }
dl-cancelled = Cancelado
dl-show-all = Mostrar todas las descargas
dl-open-folder = Abrir la carpeta Descargas

## Avisos
toast-zoom = Zoom { $pct } %  ·  Ctrl+0 lo restablece
toast-printed = { $n ->
    [one] Se envió 1 página a la impresora
   *[other] Se enviaron { $n } páginas a la impresora
}
toast-pdf-saved = { $file } ({ $n ->
    [one] 1 página
   *[other] { $n } páginas
}) guardado en Descargas
toast-pdf-no-printer = Ninguna impresora respondió; { $file } ({ $n ->
    [one] 1 página
   *[other] { $n } páginas
}) guardado en Descargas
toast-pdf-failed = No se pudo guardar el PDF: { $error }

## Ventana y pestañas
window-loading = Cargando { $target } — Ironswift
private-badge = Privada

## Buscar en la página
find-none = Sin coincidencias
find-count = { $current } de { $total }
find-hint = Intro: siguiente  ·  Mayús+Intro: anterior  ·  Esc: cerrar

## Páginas de error
err-try-again = Reintentar
err-settings = Ajustes
err-details = Detalles
err-dns-title = No se encuentra este sitio
err-dns-detail = No se encontró ninguna dirección para { $host }.
err-dns-tip1 = Comprueba que la dirección esté bien escrita.
err-dns-tip2 = Comprueba que tienes conexión a internet.
err-dns-tip3 = Puede que el sitio ya no exista.
err-timeout-title = El sitio tardó demasiado en responder
err-timeout-detail = { $host } no respondió a tiempo.
err-timeout-tip1 = Vuelve a intentarlo en un momento; puede que el sitio esté ocupado.
err-net-tip = Comprueba tu conexión a internet.
err-cert-title = Esta conexión no es segura
err-cert-detail = { $host } envió un certificado de seguridad en el que Ironswift no puede confiar, así que la página no se cargó. Alguien podría estar intentando leer o cambiar lo que envías.
err-cert-tip1 = Si es tu sitio o el de tu empresa, puede que su certificado haya caducado o no esté instalado.
err-cert-tip2 = No escribas contraseñas ni datos de pago en este sitio.
err-refused-title = El sitio rechazó la conexión
err-refused-detail = { $host } no acepta conexiones.
err-refused-tip1 = Puede que el sitio esté caído o no funcione en este puerto.
err-refused-tip2 = Vuelve a intentarlo más tarde.
err-proxy-title = No se pudo conectar a través del proxy
err-proxy-detail = El servidor proxy no dejó pasar la solicitud.
err-proxy-tip = Revisa el proxy en Ajustes (Red).
err-type-title = Ironswift no puede mostrar este archivo
err-type-detail = Este tipo de archivo no se puede mostrar en una pestaña.
err-other-title = No se pudo cargar esta página
err-other-detail = Algo salió mal al cargar { $host }.
err-other-tip = Vuelve a intentarlo con ↻.

## Ajustes
set-language = Idioma
set-language-auto = El mismo que el escritorio
set-language-hint = Para los menús y mensajes de Ironswift, y el idioma que se comunica a los sitios. Se aplica por completo la próxima vez que se inicie Ironswift.
set-high-contrast = Alto contraste
set-high-contrast-hint = Muestra las páginas y el navegador en negro, blanco y amarillo, sin imágenes de fondo, para leer con más facilidad.

## Screen readers (window, tabs, toolbar) and the settings page
a11y-tabs = Pestañas
a11y-toolbar = Navegación
a11y-address = Barra de direcciones y búsqueda
tab-untitled = Pestaña nueva
button-back = Atrás
button-forward = Adelante
button-reload = Recargar
settings-title = Ajustes
settings-sub = Se guardan solo en este equipo.
settings-saved = Ajustes guardados.
settings-cache-cleared = Archivos en caché borrados ({ $mb } MB liberados).
settings-startup = Inicio y búsqueda
settings-home = Página de inicio
settings-search-engine = Buscador
settings-restore-tabs = Volver a abrir las pestañas de la última vez
settings-bookmarks-bar = Mostrar la barra de marcadores
settings-memory = Memoria
settings-sleep = Dormir las pestañas en segundo plano tras (minutos, 0 = nunca)
settings-sleep-hint = Una pestaña dormida libera su página y se recarga al volver a ella.
settings-gpu = Dibujar con la tarjeta gráfica
settings-gpu-hint = Desplazamiento más suave y menos trabajo para el procesador; el controlador gráfico usa unos 40–70 MB más de memoria. Se aplica la próxima vez que se inicie Ironswift.
settings-site-processes = Aislamiento de sitios: cada sitio en su propio proceso
settings-site-processes-hint = Las páginas de cada sitio se ejecutan en un proceso aparte y restringido, sin acceso a tus archivos, a las cookies de otros sitios ni directamente a la red. Usa más memoria. Buscar en la página, la selección de texto y algunos menús aún no funcionan en esas páginas. Se aplica la próxima vez que se inicie Ironswift.
settings-privacy = Privacidad
settings-tpc = Compartir cookies con sitios incrustados
settings-tpc-hint = Desactivado por defecto: un sitio incrustado en otros (una red de anuncios o de rastreo, un reproductor de vídeo) recibe un almacén de cookies distinto en cada sitio, así no puede seguirte.
settings-strip = Quitar etiquetas de rastreo de los enlaces
settings-strip-hint = utm_source, fbclid, gclid y similares.
settings-gpc = Enviar Global Privacy Control
settings-gpc-hint = Pide a los sitios que no vendan ni compartan tus datos.
settings-clear-exit = Borrar cookies y datos de sitios al cerrar Ironswift
settings-clear-exit-hint = Cierra tu sesión en los sitios cada vez; el historial y los marcadores se conservan.
settings-cookies-allow = Permitir siempre las cookies de estos sitios
settings-cookies-allow-hint = Incluso cuando están incrustados en otros sitios (para inicios de sesión o comentarios que las necesitan).
settings-cookies-block = No permitir nunca las cookies de estos sitios
settings-cookies-session = Borrar las cookies de estos sitios al cerrar Ironswift
settings-partition-hint = Los demás sitios incrustados en una página reciben su propio almacén de cookies para esa página, así no pueden seguirte de un sitio a otro.
settings-cookie-sites = Sitios con cookies ({ $count })
settings-clear-data = Borrar ahora los datos de navegación
settings-clear-data-hint = Historial, cookies, datos de sitios y archivos en caché. Los marcadores y ajustes se quedan.
settings-private-hint = Para navegar sin guardar nada, abre una ventana privada con Ctrl+Mayús+N.
settings-blocking = Bloqueo y seguridad
settings-block-ads = Bloquear anuncios
settings-block-ads-hint = EasyList: se detienen las peticiones de anuncios y se ocultan los huecos vacíos. Las páginas cargan más rápido.
settings-block-trackers = Bloquear rastreadores
settings-block-trackers-hint = EasyPrivacy: analíticas, píxeles de rastreo y scripts de huella digital.
settings-no-blocking = Desactivar el bloqueo en estos sitios
settings-safe = Avisarme antes de sitios peligrosos
settings-safe-hint = Sitios de la lista de malware URLhaus y de la lista Phishing Army. La comprobación se hace en este equipo; no se envía nada.
settings-rfp = Resistir la huella digital
settings-rfp-hint = Los sitios ven un tamaño de pantalla común, las fuentes estándar y lecturas de canvas algo variadas, así no pueden reconocer tu equipo por sus detalles.
settings-update-lists = Actualizar las listas ahora
settings-permissions-hint = Cámara, micrófono, ubicación y notificaciones: los sitios preguntan primero.
settings-updates = Actualizaciones
settings-check-updates = Buscar versiones nuevas una vez al día
settings-check-now = Buscar ahora
settings-appearance = Apariencia
settings-min-font = Tamaño mínimo de letra (px, 0 = ninguno)
settings-min-font-hint = Agranda la letra pequeña de las páginas. Ctrl + y Ctrl − amplían todo un sitio.
settings-reduce-motion = Reducir el movimiento
settings-reduce-motion-hint = Pide a los sitios que reduzcan al mínimo las animaciones.
settings-default-browser = Navegador predeterminado
settings-make-default = Hacer de Ironswift mi navegador predeterminado
settings-network = Red
settings-proxy = Proxy
settings-proxy-system = Ajustes del sistema
settings-proxy-none = Sin proxy
settings-proxy-manual = Usar este proxy
settings-proxy-address = Dirección del proxy (para «Usar este proxy»)
settings-proxy-hint = «Ajustes del sistema» sigue el proxy del escritorio y las variables http_proxy / https_proxy / no_proxy. Los sitios seguros se comprueban con el almacén de certificados del equipo.
settings-clear-cache = Borrar los archivos en caché
settings-cache-size = ({ $mb } MB guardados para que las páginas carguen más rápido)
settings-save = Guardar ajustes
settings-history = Historial
settings-bookmarks = Marcadores
settings-cookies-deleted = Cookies de { $site } borradas.
settings-cookies-allowed = { $site } puede usar cookies siempre.
settings-cookies-blocked = { $site } ya no puede usar cookies (se borraron las suyas).
settings-cookies-session-set = Las cookies de { $site } se borrarán al cerrar Ironswift.
settings-cookies-default = { $site } sigue las reglas normales de cookies.
settings-permissions-reset = Permisos de { $site } restablecidos.

## Find bar label
find-label = Buscar
