# Sincronización rápida

*Lea esto en otros idiomas: [English](../fast-sync.md), [简体中文](fast-sync_ZH-CN.md), [Korean](fast-sync_KR.md).*

En Grin, llamamos "sync" al proceso de sincronizar un nuevo nodo o un nodo que no ha estado al día con la cadena durante un
tiempo, y llevarlo hasta el último bloque conocido. La Descarga Inicial de Bloques (o IBD) es usada a menudo por otras cadenas
de bloques, pero esto es problemático para Grin ya que típicamente no descarga bloques completos..

En resumen, una sincronización rápida en Grin hace lo siguiente:

1. Descargar todas las cabeceras de los bloques, por trozos, en la cadena más utilizada,
   tal y como lo anuncian otros nodos.
1. Encuentre una cabecera suficientemente alejada del encabezado de la cadena. Esto se denomina horizonte de nodo, ya que es lo
   más lejos que un nodo puede reorganizar su cadena en una nueva bifurcación en caso de que ocurriera sin activar otra nueva
   sincronización completa.
1. Descargue el estado completo tal y como estaba en el horizonte, incluyendo los datos de salida no utilizados, los datos de
   pruebas de rango y del núcleo, así como todos los MMR correspondientes. Este es sólo un gran archivo zip.
1. Validar el estado total.
1. Descarga bloques completos desde el horizonte para llegar a la cabeza de la cadena.

En el resto de esta sección, nos detendremos en cada uno de estos pasos.
