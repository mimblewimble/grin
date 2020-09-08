# Einführung in Mimblewimble und Grin

*In anderen Sprachen lesen: [English](../intro.md), [Español](intro_ES.md), [Nederlands](intro_NL.md), [Русский](intro_RU.md), [日本語](intro_JP.md), [Deutsch](intro_DE.md), [Portuguese](intro_PT-BR.md), [Korean](intro_KR.md), [简体中文](intro_ZH-CN.md)*

Mimblewimble ist ein Blockchain-Format und Protokoll, welches auf starke kryptographische Primitiven setzt und dadurch äußerst gute Skalierbarkeit, Privatsphäre und Fungibilität bietet. Es befasst sich mit Lücken, die in fast allen gegenwärtigen Blockchainimplementierungen existieren.

Grin ist ein Open-Source-Softwareprojekt, dass eine Mimblewimble-Blockchain implementiert und die für den Einsatz einer vollständigen Blockchain und Kryptowährung nötigen Lücken schließt.

Das Hauptziel und die Charakteristika des Grin-Projekts sind wie folgt:

* Standardmäßige Privatsphäre. Dies ermöglicht volle Fungibilität, ohne die Fähigkeit auszuschließen, Informationen nach Bedarf selektiv preisgeben zu können.
* Skaliert hauptsächlich mit der Anzahl der Nutzer und minimal mit der Anzahl an Transaktionen (<100 byte `kernel`), was zu hoher Platzsparung im Vergleich zu anderen Blockchains führt.
* Starke und bewährte Kryptografie. Mimblewimble setzt nur auf seit Jahrzehnten erprobte Elliptische-Kurven-Kryptografie.
* Einfachheit des Designs, die das dauerhafte Auditieren und Aufrechterhalten leicht gestaltet.
* Von der Gemeinschaft gelenkt, die Dezentralisierung des Minings fördernd.

## Tongue Tying für Jedermann

Dieses Dokument richtet sich an Leser, die ein gutes Verständnis von Blockchain und grundlegender Kryptografie haben. Vor diesem Hintergrund sind wir bestrebt, den technischen Aufbau von Mimblewimble, sowie dessen Einsatz in Grin zu erklären. Wir hoffen, dass dieses Dokument für die meisten technikbegeisterten Leser verständlich ist. Unser Ziel ist es, dich für Grin zu begeistern und dein Interesse zu wecken, dich in jeder möglichen Weise einzubringen.

Um dieses Ziel zu erreichen, führen wir die für ein gutes Verständnis von Grin als Mimblewimble-Umsetzung nötigen Hauptkonzepte ein. Wir beginnen mit einer kurzen Erläutering einiger relevanter Eigenschaften der Elliptischen-Kurven-Kryptografie (ECC), um die Grundlagen für Grin zu legen und anschließend die Kernelemente von Transaktionen und Blocks im Mimblewimble-Blockchain zu beschreiben.

### Tiny Bits of Elliptic Curves

Wir beginnen mit einer kurzen Einführung in Elliptische-Kurven-Kryptografie, wobei wir nur die Eigenschaften betrachten, die für das Verständnis von Mimblewimbles Funktionsweise nötig sind, ohne die Feinheiten von ECC eingehend zu vertiefen. Für Leser, die tiefer in diese Vorraussetzungen einzutauchen wünschen, gibt es weitere Möglichkeiten [mehr zu lernen](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

Eine elliptische Kurve zum Zwecke der Kryptografie ist ein großes Set an Punkten, die wir _C_ nennen. Diese Punkte können von Integern (auch Skalare genannt) addiert, substrahiert, oder multipliziert werden. Mit einem Integer _k_ und mittels einer Operation der skalaren Multiplikation können wir `k*H` errechnen, was auch einen Punkt auf der Kurve _C_ darstellt. Mit einem weiteren Integer _j_ können wir ferner `(k+j)*H` errechnen, was `k*H + j*H` gleicht. Diese Addition- und Skalarmultiplikationsoperationen auf einer elliptischen Kurve behalten die kommutativen und assoziativen Eigenschaften der Addition und Multiplikation bei:

    (k+j)*H = k*H + j*H

Wenn wir in ECC eine sehr große Zahl _k_ als privaten Schlüssel wählen, gilt `k*H` als der korrespondierende öffentliche Schlüssel. Selbst wenn der Wert des öffentlichen Schlüssels `k*H` bekannt ist, ist die Ableitung von _k_ nahezu unmöglich (oder anders ausgedrückt, während die Multiplikation trivial ist, ist die "Division" durch Kurvenpunkte extrem schwierig).

Die vorherige Formel `(k+j)*H = k*H + j*H`, mit jeweils _k_ und _j_ als privaten Schlüsseln, demonstriert, dass ein aus der Addition zweier privater Schlüssel (`(k+j)*H`) erhaltener öffentlicher Schlüssel identisch mit der Addition der öffentlichen Schlüssel für jeden der zwei privaten Schlüssel (`k*H + j*H`) ist. In der Bitcoin-Blockchain stützen sich Hierarchical Deterministic Wallets stark auf dieses Prinzip. Gleiches gilt auch für Mimblewimble und die Grin-Implementierung.

### Transaktionen mit Mimblewimble

Die Struktur von Transaktionen veranschaulicht einen wesentlichen Grundsatz von Mimblewimble: starke Privatsphäre und Garantie der Vertraulichkeit.

Die Validierung von Mimblewimble-Transaktionen hängt von zwei grundlegenden Eigenschaften ab:

  * **Verifizierung von Zero Sums.** Die Summe der Outputs minus Inputs ergibt immer Null, was beweist, dass die Transaktion keine neuen Gelder erschaffen hat, _ohne dabei die tatsächlichen Beträge zu enthüllen._
  * **Besitz von privaten Schlüsseln.** Wie bei den meisten anderen Kryptowährungen ist das Eigentum der Transtaktionsoutputs durch den Besitz der ECC-Privatschlüssel garantiert. Jedoch wird der Beweis, dass eine Entität jene privaten Schlüssel besitzt, nicht durch das direkte Signieren der Transaktion erreicht.

Die folgenden Abschnitte über Kontostand, Besitz, Wechselgeld, und Beweise, beschreiben ausführlich wie diese zwei grundelegenden Eigenschaften erreicht werden.

#### Kontostand

Aufbauend auf die oben beschriebenen Eigenschaften von ECC, können Werte in einer Transaktion verdeckt werden.

Falls _v_ der Wert eines Transaktions-Inputs oder Outputs und _H_ eine elliptische Kurve ist, können wir einfach `v*H` statt _v_ in einer Transaktion einbetten. Dies funktioniert, da wir durch die Benutzung der ECC-Operationen dennoch validieren können, dass die Summe der Outputs einer Transaktion der Summe der Inputs gleicht:

    v1 + v2 = v3  =>  v1*H + v2*H = v3*H

Diese Eigenschaft, bei jeder Transaktion zu verifizieren, erlaubt es dem Protokoll, ohne Kenntnis über die tatsächlichen Werte zu haben, nachzuweisen, dass eine Transaktion nicht Geld aus dem Nichts erschafft. Allerdings gibt es eine endliche Zahl nutzbarer Werte, und es könnte jeder einzelne davon getestet werden, um den Wert deiner Transaktion zu schätzen. Darüber hinaus legt die Kenntnis über v1 (beispielsweise von vorherigen Transaktionen) und das daraus resultierende `v1*H`, alle Outputs mit dem Wert v1 quer über die Blockchain offen. Aus diesen Gründen führen wir eine zweite elliptische Kurve _G_ ein (praktischerweise ist _G_ nur ein weiterer Generatorpunkt auf der gleichen Kurve wie _H_), sowie einen privaten Schlüssel _r_, welcher als *Blinding Factor* genutzt wird.

Ein Input- oder Outputwert in einer Transaktion kann sodann ausgedrückt werden als:

    r*G + v*H

Wobei:

* _r_ ein privater Schlüssel ist, der als Blinding Factor genutzt wird, _G_ eine elliptische Kurve, und deren Produkt `r*G` der öffentliche Schlüssel für _r_ auf _G_ ist.
* _v_ der Wert eines Inputs oder Outputs und _H_ eine weitere elliptische Kurve ist.

Weder _v_ noch _r_ können abgeleitet werden, was die grundlegenden Eigenschaften der Elliptischen-Kurven-Kryptografie wirksam zum Einsatz bringt. `r*G + v*H` wird als ein _Pedersen Commitment_ bezeichnet.

Als Beispiel nehmen wir an, dass wir eine Transaktion mit zwei Inputs und einem Output erstellen möchten. Wir haben (die Kosten ignorierend):

* vi1 und vi2 als Inputwerte.
* vo3 als Outputwerte.

Sodass

    vi1 + vi2 = vo3

Durch die Erstellung eines privaten Schlüssels als Blinding Factor für jeden Inputwert und das Austauschen jedes Wertes mit den respektiven Pedersen Commitments der vorherigen Gleichung, erhalten wir:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vo3*H)

Was als Konsequenz vorraussetzt, dass:

    ri1 + ri2 = ro3

Dies ist der erste Grundpfeiler von Mimblewimble: die für die Validierung einer Transaktion erforderliche Arithmetik kann durchgeführt werden, ohne Kenntnis über die Werte zu haben.

Zum Schluss sei erwähnt, dass diese Idee von Greg Maxwells [Confidential Transactions](https://elementsproject.org/features/confidential-transactions/investigation) abgeleitet wurde, und jene wiederum von Adam Backs Vorschlag für an Bitcoin angepasste homomorphe Werte.

#### Besitz

In den vorherigen Abschnitten haben wir private Schlüssel als Blinding Factor, um die Transaktionswerte zu verbergen, eingeführt. Die zweite Erkenntnis von Mimblewimble ist, dass private Schlüssel zum Einsatz kommen können, um den privaten Besitz des Wertes zu beweisen.

Alice schickt dir 3 Coins. Um diesen Betrag zu verbergen, wählst du 28 als Blinding Factor (es sei angemerkt, dass der Blinding Factor als privater Schlüssel in der Praxis eine sehr große Zahl darstellt). Irgendwo auf der Blockchain erscheint der folgende Output, der nur von dir ausgebbar sein sollte:

    X = 28*G + 3*H

_X_ ist das Ergebnis der Addition und von jedem einsehbar. Der Wert 3 ist nur dir und Alice bekannt, und 28 ist nur dir bekannt.

Um diese 3 Coins erneut zu verschicken, ist es für das Protokoll nötig, dass 28 auf irgendeine Art bekannt ist. Um zu zeigen wie dies funktioniert, nehmen wir an, dass du 3 der gleichen Coins an Carol verschickst. Du musst dafür eine simple Transaktion wie die folgende erstellen:

    Xi => Y

Wobei _Xi_ ein Input ist, der deinen Output _X_ ausgibt, und Y Carols Output ist. Es gibt keinen Weg eine solche Transaktion zu erstellen und zu bi­lan­zie­ren, ohne deinen privaten Schlüssel von 28 zu kennen. In der Tat ist es so, dass Carol, falls sie die Transaktion ausbilanzieren möchte, sowohl den verschickten Wert als auch den prviaten Schlüssel wissen muss, sodass:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H

Durch Überprüfung, dass alles Null ergibt, können wir sicherstellen, dass kein neues Geld erstellt wurde.

Moment! Stop! Da du nun den privaten Schlüssel in Carols Output kennst (der in diesem Fall der Gleiche wie deiner sein muss um Gleichgewicht zu schaffen). Somit könntest du das Geld von Carol zu dir zurück stehlen!

Um dies zu lösen, nutzt Carol einen privaten Schlüssel ihrer Wahl. Sie wählt beispielsweise 113 aus, was davon beim Blockchain ankommt ist wie folgt:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H

Nun ergibt die Transaktion nicht länger Null und wir haben einen Wertüberschuss auf _G_ (85), was das Ergebnis der Summierung aller Blinding Factors ist. Weil aber `85*G` ein gültiger öffentlicher Schlüssel auf der elliptischen Kurve _C_, mit dem privaten Schlüssel 85, für jedes x und y, ist, gilt `x*G + y*H` nur dann als gültiger öffentlicher Schlüssel auf _G_, wenn `y = 0` ist.

Daher muss das Protokoll lediglich verifizieren, dass (`Y - Xi`) ein gültiger öffentlicher Schlüssel auf _G_ ist, und dass die Transaktionspartner gemeinsam den privaten Schlüssel kennen (85 in unserer Transaktion mit Carol). Der einfachste Weg dies zu tun ist eine mit dem Wertüberschuss (85) erstellte Signature zu erfordern, die dann validiert, dass:

* die Transaktionspartner gemeinsam den privaten Schlüssel kennen, und
* die Summe der Transaktionsoutputs, minus der Inputs, Null ergibt (weil nur ein gültiger öffentlicher Schlüssel, der dem privaten Schlüssel entspricht, den Abgleich mit der Signatur vollbringt).

Diese Signatur, die jeder Transaktion zusammen mit weiteren Daten (wie Mininggebühren) beigefügt ist, wird als ein _Transaction Kernel_ bezeichnet und von allen Validierern geprüft.

#### Einige Feinheiten

Dieser Abschnitt führt die Erstellung von Transaktionen weiter aus und erörtert wie Wechselgeld eingeführt wird, sowie ferner die Voraussetzung von Range Proofs, sodass alle Werte nachweislich nicht negativ sind. Keines der beiden ist für das Verständnis von Mimblewimble und Grin absolut von Nöten, falls du es also eilig hast, spring einfach gleich zur [Zusammenfassung](#zusammenfassung).

#### Wechselgeld

Angenommen, du möchtest gerne 2 der 3 von Alice empfangenen Coins an Carol schicken. Um dies zu tun, würdest du den verbleibenden 1 Coin an dich selbst als Wechselgeld zurückschicken. Du generierst einen weiteren privaten Schlüssel (beispielsweise 12) als Blinding Factor, um deinen Wechselgeldoutput zu schützen. Carol nutzt wie zuvor ihren eigenen privaten Schlüssel.

    Wechselgeldoutput:     12*G + 1*H
    Carols Output:         113*G + 2*H

Was auf die Blockchain gelangt ist sehr ähnlich wie zuvor. Die Signatur wird wieder mit dem Wertüberschuss erstellt, in diesem Falle 97.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H

##### Range Proofs

In allen obigen Berechnungen stützen wir uns darauf, dass die Transaktionswerte immer positiv sind. Die Einführung von negativen Beträgen wäre extrem problematisch, da in jeder Transaktion neue Gelder erstellt werden könnten.

Zum Beispiel könnten Transaktionen mit einem Input von 2 und Outputs von 5 und -3 erstellt werden, die trotzdem ausgeglichen sind, folgend der Definition in den vorherigen Abschnitten. Dies ist nicht einfach festzustellen, da sogar wenn _x_ Negativ ist, der korrespondierende Punkt `x.H` auf der Kurve so aussieht wie jeder andere.

Um dieses Problem zu lösen, setzt Mimblewimble ein anderes kryptographisches Konzept (ebenso stammend von Confidential Transactions) namens Range Proofs ein. Wir werden Range Proofs nicht ausführlich behandeln, du solltest nur wissen, dass wir für jedes `r.G + v.H` einen Beweis erstellen können, der zeigt, dass _v_ größer als Null ist und nicht zu Overflow führt.

Es ist auch wichtig anzumerken, dass um einen gültigen Range Proof der obigen Beispiele zu erstellen, die beiden Werte 113 und 28, die für die Erstellung und Signierung des Wertüberschusses genutzt werden, bekannt sein müssen. Der Grund dafür, sowie eine genauere Beschreibung von Range Proofs, wird im [Range Proof Paper](https://eprint.iacr.org/2017/1066.pdf) behandelt.

#### Zusammenfassung

  Eine Mimblewimble-Transaktion beinhaltet wie folgt:

  * Eine Reihe von Inputs, die referenzieren, sowie eine Reihe an vorherigen Outputs ausgeben.
  * Eine Reihe an neuen Outputs, die Folgendes umfassen:
    * einen Wert und ein Blinding Factor (welcher nur ein neuer privater Schlüssel ist) auf einer Kurve multipliziert und als `r.G + v.H` summiert.
    * Ein Range Proof der zeigt, dass v nicht negativ ist.
  * Eine eindeutige Transaktionsgebühr.
  * Eine Signatur, die sich dadurch berechnet, dass der überschüssige Blindingwert (die Summe aller Outputs plus der Gebühren, Minus der Inputs) genommen und als privater Schlüssel verwendet wird.

### Blocks und Chainstate

Wir haben oben beschrieben, wie Mimblewimble-Transaktionen starke Anonymität gewährleisten können, während die Eigenschaften für eine gültigen Blockchain beibehalten werden, das heißt, dass eine Transaktion kein Geld erstellt und der Eigentumsnachweis über private Schlüssel erfolgt.

Das Mimblewimble-Blockformat baut darauf auf, indem es ein weiteres Konzept einführt: _cut-through_. Mit dieser Erweiterung erlangt eine Mimblewimble-Blockchain:

* äußerst gute Skalierbarkeit, da die große Mehrzahl der Transaktionsdaten über Zeit entfernt werden können, ohne dabei Sicherheit zu beeinträchtigen.
* Weitergehende Anonymität durch das Vermischen und Löschen von Transaktionsdaten.
* Die Fähigkeit neuer Nodes mit dem Rest des Netzwerks sehr effizient zu synchronisieren.

#### Transaktions-Aggregation

Rufe ins Gedächtnis, dass jede Transaktion aus dem Folgenden besteht -

* Eine Reihe von Inputs, die referenzieren und eine Reihe an vorherigen Outputs ausgeben.
* Eine Reihe an neuen Outputs (Pedersen Commitments).
* Ein Transaktionskernel, bestehend aus
    * Kernel Excess (Pedersen Commitment zu Null).
    * Transaktionssignatur (mittels Kernel Excess als öffentlicher Schlüssel).

Eine Transaktion wird signiert und die Signatur in einen _Transaction Kernel_ eingefügt. Die Signatur wird durch die Nutzung des _Kernel Excess_ als öffentlicher Schlüssel erstellt, womit bewiesen wird, dass die Transaktion Null ergibt.

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

Der öffentliche Schlüssel in diesem Beispiel ist `28*G`.

Wir können sagen, dass das Folgende für jede beliebige Transaktion wahr ist (zwecks Einfachheit ungeachtet von Gebühren) -

    sum(outputs) - sum(inputs) = kernel_excess

Das gleiche gilt auch für Blocks, sobald wir realisieren, dass ein Block lediglich eine Reihe an aggregierten Inputs, Outputs, und Transaktionskerneln ist. Wir können die Transaktionsoutputs summieren, die Summe der Transaktionsinputs substrahieren, und das resultierende Pedersen Commitment mit der Summe des Kernel Excess vergleichen -

    sum(outputs) - sum(inputs) = sum(kernel_excess)

Leicht vereinfacht (weiterhin Transaktionsgebühren ignorierend) können wir sagen, dass Mimblewimble-Blocks genau wie Mimblewimble-Transaktionen behandelt werden können.

##### Kernel-Offsets

In den wie oben beschriebenen Mimblewimble-Blocks und Transaktionen gibt es ein subtiles Problem. Es ist möglich (und in manchen Fällen trivial) die konstituierende Transaktion in einem Block zu rekonstruieren. Dies ist eindeutig schlecht für die Privatsphäre. Es handelt sich um ein "subset"-Problem - bei einer gegebenen Reihe an Inputs, Outputs, und Transaktionskerneln, wird ein Subset dieser Reihe eine gültige Transaktion rekombinieren.

Beispielsweise seien die folgenden beiden Transaktionen gegeben -

    (in1, in2) -> (out1), (kern1)
    (in3) -> (out2), (kern2)

Wir können diese im folgenden Block zusammenfassen (oder aggregierte Transaktion) -

    (in1, in2, in3) -> (out1, out2), (kern1, kern2)

Es ist trivial einfach, alle möglichen Permuationen auszuprobieren, um eine der Transaktionen (die erfolgreich zu Null summiert) wiederherzustellen:

    (in1, in2) -> (out1), (kern1)

Wir wissen auch, dass alles Übrige genutzt werden kann, um die andere gültige Transaktion zu rekonstruieren -

    (in3) -> (out2), (kern2)

Um dies einzuschränken, beziehen wir einen _Kernel Offset_ in jedem Transaktionskernel mit ein. Dies ist ein Blinding Factor (privater Schlüssel) der zurück zum Kernel Excess hinzugefügt werden muss um zu verifizieren, dass die Summe der Commitments Null ergibt.

    sum(outputs) - sum(inputs) = kernel_excess + kernel_offset

Wenn wir Transaktionen in einem Block aggregieren, speichern wir ein _einzeln_ aggregiertes Offset im Blockheader. Nun haben wir ein einzelnes Offset, welches nicht in seine individuellen Transaktionskernel-Offsets zerlegt werden kann, und womit Transaktionen nicht mehr rekonstruierbar sein können -

    sum(outputs) - sum(inputs) = sum(kernel_excess) + kernel_offset

Wir "teilen" den Schlüssel `k` in `k1+k2` während des Aufbaus der Transaktion. Für einen Transaktionskernel `(k1+k2)*G` veröffentlichen wir `k1*G` (den Überschuss) und `k2` (das Offset), und signieren die Transaktion mit `k1*G` wie zuvor. Während des Blockaufbaus können wir einfach die `k2`-Offsets summieren um ein einzeln zusammengefasstes `k2`-Offset zu generieren, das alle Transaktionen in einem Block abdeckt.

#### Cut-through

Blocks erlauben es Minern multiple Transaktionen in einem einzelnen Set zusammenzustellen, welches der Chain hinzugefügt wird. In den folgenden Blockrepräsentationen, die 3 Transaktionen enthalten, zeigen wir nur Inputs und Outputs der Transaktionen. Inputs referenzieren die Outputs, die sie ausgeben.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

Wir beobachten die folgenden zwei Eigenschaften:

* Innerhalb dieses Blocks werden einige Outputs direkt von einbezogenen Inputs (I3 gibt 02 aus und I4 gibt 03 aus) ausgegeben.
* Die Struktur jeder Transaktion tut nichts zur Sache. Da alle Transaktionen individuell Null ergeben, muss die Summe aller Transaktions-Inputs und Outputs Null sein.

Ähnlich einer Transaktion ist alles, was in einem Block geprüft werden muss, dass der Besitz bewiesen ist (was von _Transaction Kernels_ kommt) und dass der gesamte Block keine Geldmenge hinzugefügt hat (außer dem, was von der Coinbase erlaubt ist). Daher können übereinstimmende Inputs und Outputs entfernt werden, da sich ihre Beteiligung an der Gesamtsumme aufhebt. Dies führt zum folgenden, viel kompakteren Block:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Vermerke, dass alle Transaktionsstrukturen entfernt wurden und die Anordnung von Inputs und Outputs nicht mehr wichtig ist. Die Summe aller Outputs in diesem Block, minus der Inputs, ist jedoch noch immer garantiert Null.

Ein Block ist einfach ausgebaut aus:

* Einem Blockheader.
* Der Liste an Inputs, die nach dem Cut-through übrig bleiben.
* Der Liste an Outputs, die nach dem Cut-through übrig bleiben.
* Ein einzelnes Kerneloffset, um den vollständigen Block zu umfassen.
* Die Transaktionskernel, die für jede Transaktion beinhalten:
    * Den öffentlichen Schlüssel `r*G`, der aus der Summierung aller Commitments erhalten wird.
    * Die Signaturen die durch die excess value generiert werden.
    * Die Mininggebühr.

Sofern so strukturiert, bietet ein Mimblewimble-Block äußerst gute Garantie der Vertraulichkeit:

* Intermediäre (cut-through) Transaktionen werden nur von ihren Transaktionskerneln repräsentiert.
* Alle Outputs sehen gleich aus: nur sehr große Zahlen, die unmöglich voneinander differenzierbar sind. Um einige Outputs auszuschließen, müssten alle ausgeschlossen werden.
* Alle Transaktionsstrukturen wurden entfernt, was es unmöglich macht zu ermitteln, welches Output mit jedem Input verbunden wurde.

Und doch validiert es alles noch immer!

#### Cut-through All The Way

Bezug nehmend auf den vorherigen Beispielblock, müssen die Outputs x1 und x2, ausgegeben von I1 und I2, zuvor auf der Blockchain aufgetreten sein. Nach der Addition dieses Blocks können jene Outputs, sowie I1 und I2, auch vom gesamten Chain entfernt werden, da sie nicht zur Gesamtsumme beitragen.

Verallgemeinernd können wir schlussfolgern, dass der Chainstate (ausgenommen Header) zu jedem Zeitpunkt durch lediglich die folgenden Informationsstücke zusammengefasst werden kann:

1. Die Gesamtanzahl an Coins, die durch Mining in der Chain erstellt wurden.
1. Das komplette Set nicht verwendeter Outputs.
1. Die Transaktionskernel für jede Transaktion.

Das erste Informationsstück kann nur mittels der Blockhöhe (seiner Distanz zum Genesisblock), abgeleitet werden. Beide nicht verwendeten Outputs und die Transaktionskernel sind höchst kompakt. Dies hat 2 wichtige Konsequenzen:

* Der Zustand, den eine gegebene Node in einer Mimblewimble-Blockchain aufrechterhalten muss, ist sehr klein (von etwa einigen Gigabytes für eine Blockchain in der Größe von Bitcoin, und potentiall optimierbar auf wenige hundert Megabytes).
* Wenn eine neue Node einem neuen Netzwerk beitritt, dass eine Mimblewimble-Chain aufbaut, ist die Menge an Informationen, die transferiert werden müssen, ebenfalls sehr klein.

Darüber hinaus kann das vollständige Set an nicht verwendeten Outputs nicht manipuliert werden, selbst nicht durch das Hinzufügen oder Entfernen eines Outputs. Würde dies getan werden, führe es dazu, dass die Summierung aller Blinding Factors in den Transaktionskerneln von der Summierung der Blinding Factors in den Outputs abweichen würde.

### Fazit

In diesem Dokument haben wir die grundlegenden Prinzipien abgedeckt, die einer Mimblewimble-Blockchain unterliegen. Durch die Nutzung von Additionseigenschaften der Elliptischen-Kurven-Kryptografie können wir Transaktionen erstellen, die völlig undurchsichtig sind, aber dennoch korrekt validiert werden können. Durch die Verallgemeinerung dieser Eigenschaften auf Blocks, können wir große Mengen an Blockchaindaten entfernen, was hohe Skalierbarkeit und schnelle Synchronisierung neuer Peers erlaubt.
