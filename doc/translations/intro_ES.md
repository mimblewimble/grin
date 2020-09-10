# Introducción a Mimblewimble y Grin

*Lea esto en otros idiomas: [English](../intro.md), [Español](intro_ES.md), [Nederlands](intro_NL.md), [Русский](intro_RU.md), [日本語](intro_JP.md), [Deutsch](intro_DE.md), [Portuguese](intro_PT-BR.md), [Korean](intro_KR.md), [简体中文](intro_ZH-CN.md).*

Mimblewimble es un formato y un protocolo de cadena de bloques que proporciona una escalabilidad, privacidad y funcionalidad
extremadamente buenas al basarse en fuertes algoritmos criptográficos. Aborda los vacíos existentes en casi todas las
implementaciones actuales de cadenas de bloques.

Grin es un proyecto de software de código abierto que implementa una cadena de bloques Mimblewimble y rellena los espacios
necesarios para una implementación completa de la cadena de bloques y moneda criptográfica.

El objetivo principal y las características del proyecto Grin son:

* Privacidad por defecto. Esto permite una funcionalidad completa sin excluir la posibilidad de revelar información de forma
  selectiva cuando sea necesario.
* Se escala principalmente con el número de usuarios y mínimamente con el número de transacciones (`<100 bytes kernel`), lo
  que resulta en un gran ahorro de espacio en comparación con otras cadenas de bloques.
* Criptografía robusta y probada. Mimblewimble sólo se basa en la criptografía de curvas elípticas que ha sido probada y
  comprobada durante décadas.
* Simplicidad de diseño que facilita la auditoría y el mantenimiento a lo largo del tiempo.
* Dirigido por la comunidad, utilizando un algoritmo de minería resistente a la ASICs (Cuckoo Cycle) que fomenta la
  descentralización de la minería.

## Tongue Tying para todos

Este documento está dirigido a lectores con un buen conocimiento de cadenas de bloques y de la criptografía básica. Con
esto en mente, tratamos de explicar el desarrollo técnico de Mimblewimble y cómo se aplica en Grin. Esperamos que este
documento sea comprensible para la mayoría de los lectores con visión técnica. Nuestro objetivo es animarles a interesarse en
Grin y contribuir de cualquier manera posible.

Para lograr este objetivo, presentaremos los principales conceptos necesarios para una buena comprensión de Grin como
implementación de Mimblewimble. Comenzaremos con una breve descripción de algunas propiedades relevantes de la Criptografía
de Curva Elíptica (ECC) para sentar las bases sobre las que se basa Grin y luego describir todos los elementos clave de las
transacciones y bloques de una cadena de bloques Mimblewimble.

### Pequeños Bits de Curvas Elípticas

Comenzamos con una breve introducción a la Criptografía de Curva Elíptica, revisando sólo las propiedades necesarias para
entender cómo funciona MimblewimbleWimble y sin profundizar demasiado en las complejidades de ECC. Para los lectores que
deseen profundizar en estos supuestos, existen otras opciones para [aprender más](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

Una curva elíptica con el objetivo de criptografía es simplemente un gran conjunto de puntos que llamaremos _C_. Estos puntos
pueden sumarse, restarse o multiplicarse por números enteros (también llamados escalares). Dado un entero _k_ y
usando la operación de multiplicación escalar podemos calcular `k*H`, que es también un punto en la curva _C_. Dado otro
entero _j_ también podemos calcular `(k+j)*H`, que es igual a `k*H + j*H`. Las operaciones de suma y multiplicación escalar
en una curva elíptica mantienen las propiedades conmutativas y asociativas de suma y multiplicación:

    (k+j)*H = k*H + j*H

En ECC, si escogemos un número muy grande _k_ como clave privada, `k*H` se considera la clave pública correspondiente.
Incluso si uno conoce el valor de la clave pública `k*H`, deducir _k_ es casi imposible (o dicho de otra manera, mientras que
la multiplicación es trivial, la "división" por puntos de curva es extremadamente difícil).

La fórmula anterior `(k+j)*H = k*H + j*H`, con _k_ y _j_ ambas claves privadas, demuestra que una clave pública obtenida de
la adición de dos claves privadas (`(k+j)*H`) es idéntica a la adición de las claves públicas para cada una de esas dos
claves privadas (`k*H + j*H`). En la cadena de bloques Bitcoin, las carteras jerárquicas deterministas se basan en gran
medida en este principio. Mimblewimble y la implementación de Grin también lo hacen.

### Transacciones con Mimblewimble

La estructura de las transacciones demuestra un principio crucial de Mimblewimble:
fuertes garantías de privacidad y confidencialidad.

La validación de las transacciones de MimblewimbleWimble se basa en dos propiedades básicas:

* **Verificación de importes nulos.** La suma de las salidas menos las entradas siempre es igual a cero, lo que demuestra que
  la transacción no creó nuevos fondos, _sin revelar los importes reales_.
* **Posesión de las claves privadas.** Como con la mayoría de las otras monedas criptográficas, la propiedad de los
  resultados de las transacciones está garantizada por la posesión de claves privadas ECC. Sin embargo, la prueba de que una
  entidad es propietaria de esas claves privadas no se consigue firmando directamente la transacción.

Las siguientes secciones sobre el saldo, la propiedad, el intercambio y las verificaciones detallan cómo se logran esas dos
propiedades fundamentales.

#### Balance

Basándose en las propiedades de ECC que hemos descrito anteriormente, uno puede ocultar los valores en una transacción.

Si _v_ es el valor de una transacción de entrada o salida y _H_ una curva elíptica, podemos simplemente insertar `v*H` en
lugar de _v_ en una transacción. Esto funciona porque usando las operaciones ECC, todavía podemos validar que la suma de las
salidas de una transacción es igual a la suma de las entradas:

    v1 + v2 = v3  =>  v1*H + v2*H = v3*H

Verificar esta propiedad en cada transacción permite que el protocolo verifique que una transacción no crea dinero de la
nada, sin saber cuáles son los valores reales. Sin embargo, hay un número finito de valores útiles y uno podría intentar cada
uno de ellos para adivinar el valor de su transacción. Además, conocer v1 (de una transacción anterior por ejemplo) y el
resultado `v1*H` revela todas las salidas con valor v1 a lo largo de la cadena de bloques. Por estas razones, introducimos
una segunda curva elíptica _G_ (prácticamente _G_ es sólo otro punto del generador en el mismo grupo de curvas que _H_) y una
clave privada _r_ utilizada como *factor de ocultación*.

Un valor de entrada o de salida en una operación puede expresarse como:

    r*G + v*H

Donde:

* _r_ es una clave privada utilizada como factor de ocultación, _G_ es una curva elíptica y su producto `r*G` es la clave
pública para _r_ en _G_.
* _v_ es el valor de una entrada o salida y _H_ es otra curva elíptica.

No se puede deducir ni _v_ ni _r_, aprovechando las propiedades fundamentales de la criptografía de curva elíptica. `R*G + v*H` se llama _Compromiso Pedersen_.

Por ejemplo, supongamos que queremos construir una transacción con dos entradas y una salida. Tenemos (ignorando las cuotas):

* vi1 and vi2 como valores de entrada.
* vo3 como valores de salida.

De tal forma que:

    vi1 + vi2 = vo3
Generando una clave privada como factor de ocultación para cada valor de entrada y sustituyendo cada valor por sus
respectivos Compromisos de Pedersen en la ecuación anterior, obtenemos:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vo3*H)

Lo cual, requiere como consecuencia que:

    ri1 + ri2 = ro3

Este es el primer pilar de Mimblewimble: la aritmética necesaria para validar una transacción se puede hacer sin conocer
ninguno de los valores.

Como nota final, esta idea se deriva en realidad de Greg Maxwell's
[Transacciones confidenciales](https://elementsproject.org/features/confidential-transactions/investigation),
que a su vez se deriva de una propuesta de Adam Back para valores homomórficos aplicados a Bitcoin.

#### Propiedad

En la sección anterior introducimos una clave privada como factor de ocultación para cubrir los valores de la transacción. La
segunda idea de Mimblewimble es que esta clave privada puede ser utilizada para probar la propiedad del valor.

Alice te envía 3 monedas y para ocultar esa cantidad, elegiste 28 como tu factor de ocultación (nota que en la práctica,
siendo el factor de ocultación una llave privada, es un número extremadamente grande). En algún lugar de la cadena de
bloques, la siguiente salida aparece y sólo debe ser utilizable por usted:

    X = 28*G + 3*H

_X_, el resultado de la suma, es visible para todos. El valor 3 sólo lo conocen usted y Alice, y el valor 28 sólo lo conocerá usted.

Para volver a transferir esas 3 monedas, el protocolo exige que se conozcan de alguna manera. Para demostrar cómo funciona
esto, digamos que usted quiere transferir esas 3 mismas monedas a Carol. Es necesario construir una transacción simple como
esa:

    Xi => Y

Donde _Xi_ es una entrada que utiliza su salida _X_ e Y es la salida de Carol. No hay manera de construir tal transacción y
equilibrarla sin conocer su clave privada de 28. De hecho, si Carol va a equilibrar esta transacción, necesita saber tanto el
valor enviado como su clave privada para poder hacerlo:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H

Comprobando que todo ha sido puesto a cero, podemos asegurarnos de nuevo de que no se ha creado ningún dinero nuevo.

Espera! para! ahora ya conoces la clave privada en la salida de Carol (que, en este caso, debe ser la misma que la tuya para
equilibrarla) y así podrías robarle el dinero a Carol!

Para resolver esto, Carol utiliza una clave privada de su elección.
Ella escoge 113, y lo que termina en la cadena es:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H

Ahora la transacción ya no suma cero y tenemos un _exceso de valor_ en _G_ (85), que es el resultado de la suma de todos los
factores de ocultamiento. Pero porque `85*G` es una clave pública válida en la curva elíptica _C_, con clave privada 85, para
cualquier x e y, sólo si `y = 0` es `x*G + y*H` una clave pública válida en _G_.

Así que todo lo que el protocolo necesita verificar es que (`Y - Xi`) es una clave pública válida en _G_ y que las partes que
realizan la transacción conocen en conjunto la clave privada (85 en nuestra transacción con Carol). La forma más sencilla de
hacerlo es requerir una firma construida con el valor excedente (85), que luego lo valida:

* Las partes que realizan la transacción conocen colectivamente la clave privada, y
* La suma de las salidas de transacción, menos las entradas, suma a un valor cero (porque sólo una clave pública válida, que
coincida con la clave privada, se comprobará con la firma)

Esta firma, que se adjunta a cada transacción, junto con algunos datos adicionales (como las tasas de explotación minera), se
denomina  _transacción de kernel_ y es comprobada por todos los validadores.

#### Algunos puntos más precisos

Esta sección explica con más detalle la creación de transacciones discutiendo cómo se introduce el cambio y el requisito de pruebas de rango para que se demuestre que todos los valores no son negativos. Ninguno de los dos es absolutamente necesario para entender MimblewimbleWimble y Grin, así que si tienes prisa, no dudes en ir directamente a
[Poniendo todo junto](https://github.com/wimel/grin/blob/master/doc/intro.md#putting-it-all-together).

##### Cambio

Digamos que sólo quieres enviar 2 monedas a Carol de las 3 que recibiste de Alice. Para hacer esto enviarías la moneda
restante a ti mismo como cambio. Se genera otra clave privada (digamos 12) como un factor oculto para proteger la salida de
modificación. Carol usa su propia clave privada como antes.

    Change output:     12*G + 1*H
    Carol's output:    113*G + 2*H

Lo que termina en la cadena de bloques es algo muy similar a lo que había antes.
Y la firma se construye de nuevo con el valor excedente, 97 en este ejemplo.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H

##### Pruebas de recorrido

En todos los cálculos anteriores, confiamos en que los valores de las transacciones sean siempre positivos. La introducción
de cantidades negativas sería extremadamente problemática, ya que se podrían crear nuevos fondos en cada transacción.

Por ejemplo, se podría crear una transacción con una entrada de 2 y salidas de 5 y -3 y aún así obtener una transacción bien
equilibrada, siguiendo la definición de las secciones anteriores. Esto no puede ser fácilmente detectado porque incluso si
_x_ es negativo, el punto correspondiente `x.H` en la curva se ve como cualquier otro.

Para resolver este problema, Mimblewimble utiliza otro concepto criptográfico (también procedente de Transacciones
Confidenciales) llamado pruebas de rango: una prueba de que un número está dentro de un rango dado, sin revelar el número. No
daremos más detalles sobre la prueba de rango, pero sólo necesitas saber que para cualquier `r.G + v.H` podemos construir una
prueba que demuestre que _v_ es mayor que cero y no se sobrecarga.

También es importante tener en cuenta que para crear una prueba de rango válida a partir del ejemplo anterior, deben
conocerse tanto los valores 113 como 28 utilizados al crear y firmar el exceso de valor. La razón de ello, así como una
descripción más detallada de las pruebas de rango, se detallan en la sección [range proof paper](https://eprint.iacr.org/2017/1066.pdf).

#### Poniendo todo junto

Una transacción Mimblewimble incluye lo siguiente:

* Un conjunto de entradas, que hacen referencia y consumen un conjunto de salidas anteriores.
* Un conjunto de nuevos resultados que incluyen:
* Un valor y un factor oculto (que es sólo una nueva clave privada) multiplicados en una curva
  y sumados para ser `r.G + v.H`.
* Una prueba de rango que muestra que v no es negativo.
* Una comisión explícita de transacción, en compensación..
* Una firma, que se calcula tomando el exceso de valor oculto (la suma de todas las salidas más la tarifa, menos las
  entradas) y utilizándolo como clave privada.

### Bloques y estado de la cadena

Hemos explicado anteriormente cómo las transacciones de Mimblewimble pueden proporcionar fuertes garantías de anonimato a la
vez que mantienen las propiedades requeridas para una cadena de bloques válida, es decir, una transacción no crea dinero y la
prueba de la propiedad se establece a través de claves privadas.

El formato de bloques Mimblewimble se basa en esto introduciendo un concepto adicional: _cut-through_. Con esta incorporación, una cadena Mimblewimble gana:

* Extremadamente buena escalabilidad, ya que la gran mayoría de los datos de las transacciones pueden ser eliminados con el
  tiempo, sin comprometer la seguridad.
* Mayor anonimato al mezclar y eliminar datos de transacciones.
* Y la capacidad de los nuevos nodos para sincronizarse con el resto de la red de forma muy eficiente.

#### Agrupación de transacciones

Recuerde que una transacción consiste en lo siguiente -

* un conjunto de entradas que hacen referencia y gastan un conjunto de realizaciones anteriores
* un conjunto de nuevos resultados (compromisos de Pedersen)
* un kernel de transacción, que consta de
  * kernel excess (compromiso de Pedersen a cero).
  * firma de la transacción (usando el exceso de kernel como clave pública).

Se firma una tx y se incluye la firma en un _transaction  kernel_. La firma se genera utilizando el _kernel excess_ como clave pública, lo que prueba que la transacción suma 0.

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

La clave pública en este ejemplo es `28*G`.

Podemos decir que lo siguiente es cierto para cualquier transacción válida (ignorando los cargos por simplicidad) -

    sum(outputs) - sum(inputs) = kernel_excess

Lo mismo ocurre con los bloques mismos una vez que nos damos cuenta de que un bloque es simplemente un conjunto de entradas,
salidas y núcleos de transacción agregados. Podemos sumar las salidas tx, restar la suma de las entradas tx y comparar el
compromiso resultante de Pedersen con la suma de los excesos del núcleo. -

    sum(outputs) - sum(inputs) = sum(kernel_excess)

Simplificando un poco, (de nuevo ignorando las tarifas de transacción) podemos decir que los bloques Mimblewimble pueden ser
tratados exactamente como transacciones Mimblewimble.

#### Kernel Offsets

Hay un problema leve con los bloques y las transacciones de Mimblewimble como se describe anteriormente. Es posible (y en
algunos casos trivial) reconstruir las transacciones constituyentes en un bloque. Esto es claramente malo para la privacidad.
Este es el problema del "subconjunto" - dado un conjunto de entradas, salidas y núcleos de transacción, un subconjunto de
estos se recombinará para reconstruir una transacción válida.

Por ejemplo, teniendo en cuenta las dos transacciones siguientes -

    (in1, in2) -> (out1), (kern1)
    (in3) -> (out2), (kern2)

Podemos agregarlos en el siguiente bloque (o transacción agregada) -

    (in1, in2, in3) -> (out1, out2), (kern1, kern2)

Es relativamente fácil intentar todas las permutaciones posibles para recuperar una de las transacciones
(donde se suma con éxito a cero). -

    (in1, in2) -> (out1), (kern1)

También sabemos que todo lo que queda puede ser usado para reconstruir la otra transacción válida.-

    (in3) -> (out2), (kern2)

Para mitigar esto, incluimos un _kernel offset_ con cada transacción del kernel. Este es un factor oculto (clave privada) que
debe añadirse de nuevo al exceso de núcleo para verificar que la suma de los compromisos sea cero. -

    sum(outputs) - sum(inputs) = kernel_excess + kernel_offset

Cuando agregamos transacciones en un bloque, almacenamos un único offset de agregados en el encabezado del bloque. Y ahora
tenemos una única compensación que no puede descomponerse en las compensaciones del núcleo de transacciones individuales y
las transacciones ya no pueden reconstruirse. -

    sum(outputs) - sum(inputs) = sum(kernel_excess) + kernel_offset

"Dividimos" la clave `k` en `k1+k2` durante la construcción de la transacción. Para una transacción kernel `(k1+k2)*G` publicamos `k1*G` (el exceso) y `k2` (el offset) y firmamos la transacción con `k1*G` como antes.
Durante la construcción del bloque podemos simplemente sumar las compensaciones `k2` para generar una sola compensación
agregada `k2` para cubrir todas las transacciones en el bloque. La compensación `k2` para cualquier transacción individual es
imposible de recuperar.

#### Cortado a medida

Los bloques permiten a los mineros ensamblar múltiples transacciones en un solo conjunto que se añade a la cadena. En las
siguientes representaciones en bloque, que contienen 3 transacciones, sólo se muestran las entradas y salidas de las
transacciones. Los ingresos hacen referencia a las salidas que gastan. Una salida incluida en un bloque anterior está marcada
con una x minúscula.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

Observamos las dos propiedades siguientes:

* Dentro de este bloque, algunas salidas son gastadas directamente por las entradas incluidas (I3 gasta O2 e I4 gasta O3).
* La estructura de cada transacción no importa realmente. Como todas las transacciones se suman individualmente a cero, la
  suma de todas las entradas y salidas de transacciones debe ser cero.

Al igual que en una transacción, todo lo que hay que comprobar en un bloque es que la propiedad ha sido probada (que proviene
de _núcleos de transacción_) y que todo el bloque no ha añadido ninguna cantidad de fondos (aparte de lo que permite la base
de datos de la moneda).
Por lo tanto, se pueden eliminar las entradas y salidas que coincidan, ya que su contribución a la suma total se anula. Lo
que conduce a los siguientes bloques, mucho más compactos:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Tenga en cuenta que se ha eliminado toda la estructura de transacciones y que el orden de las entradas y salidas ya no
importa. Sin embargo, la suma de todas las salidas de este bloque, menos las entradas, sigue siendo cero.

Un bloque se contruye simplemente a partir de:

* Una cabecera de bloque.
* La lista de entradas que quedan después del corte.
* La lista de las salidas que quedan después del corte.
* Un único offset del kernel para cubrir todo el bloque.
* Los núcleos de transacción que contienen, para cada transacción:
  * La clave pública `r*G` obtenida de la suma de todos los pagos.
  * Las firmas creadas utilizando el exceso de valor.
  * La tasa minera.

Con esta estructura, un bloque Mimblewimble ofrece unas garantías de privacidad muy buenas:

* Las transacciones intermedias (cut-through) estarán representadas únicamente por sus núcleos de transacciones.
* Todas las salidas se ven iguales: sólo números muy grandes que son imposibles de diferenciar entre sí. Si uno quisiera
  excluir algunas salidas, tendrían que excluir todas.
* Se ha eliminado toda la estructura de la transacción, lo que hace imposible saber qué salida se correspondía con cada
  entrada.

Y sin embargo, todo esto sigue confirmándose!

#### Atravesado todo el camino

Volviendo al bloque de ejemplo anterior, las salidas x1 y x2, gastadas por I1 e I2, deben haber aparecido previamente en la
cadena de bloques. Así que después de la adición de este bloque, esas salidas, así como I1 e I2, también pueden ser
eliminadas de la cadena global, ya que no contribuyen a la suma total.

Concluimos que, generalizando, el estado de la cadena (excluyendo las cabeceras) en cualquier momento puede ser resumido sólo
por estas piezas de información:

1. La cantidad total de monedas creadas por la minería en la cadena.
1. El conjunto completo de resultados no utilizados.
1. Los núcleos de transacciones para cada transacción.

La primera información se puede deducir simplemente usando la altura del bloque (su distancia del bloque génesis). Y tanto
las salidas no utilizadas como los núcleos de transacción son extremadamente compactos. Esto tiene dos consecuencias
importantes:

* El estado que un nodo dado en una cadena de bloques Mimblewimble necesita mantener es muy pequeño (del orden de unos pocos
  gigabytes para una cadena del tamaño de Bitcoin, y potencialmente configurable a unos pocos centenares de megabytes).
* Cuando un nuevo nodo se une a una red formando una cadena Mimblewimble, la cantidad de información que necesita ser
  transferida es también muy pequeña..

Además, el conjunto completo de resultados no utilizados no puede ser alterado, ni siquiera añadiendo o quitando un
resultado. Hacerlo haría que la suma de todos los factores de ocultación en los núcleos de la transacción difiriera de la
suma de la ocultación, factores que influyen en los resultados.

### Conclusión

En este documento tratamos los principios básicos que subyacen a una cadena de bloques Mimblewimble. Utilizando las
propiedades de suma de la Criptografía de Curva Elíptica, somos capaces de construir transacciones que son completamente
opacas pero que todavía pueden ser validadas adecuadamente. Y al generalizar esas propiedades a bloques, podemos eliminar una
gran cantidad de datos de la cadena de bloques, lo que permite una gran escalabilidad y una sincronización rápida de nuevos
participantes.
