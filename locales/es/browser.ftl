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
