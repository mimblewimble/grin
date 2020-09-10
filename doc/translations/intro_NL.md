# Inleiding tot Mimblewimble en Grin

*Lees dit in andere talen: [English](../intro.md), [Español](intro_ES.md), [Nederlands](intro_NL.md), [Русский](intro_RU.md), [日本語](intro_JP.md), [Deutsch](intro_DE.md), [Portuguese](intro_PT-BR.md), [Korean](intro_KR.md), [简体中文](intro_ZH-CN.md).*

Mimblewimble is een blockchain formaat en protocol die extreem goede schaalbaarheid, privacy en fungibiliteit biedt door zich te berusten op sterke cryptografische primiteven. Het adresseert de lacunes die in bijna alle huidige blockchain-implementaties bestaan.

Grin is een open source softwareproject dat een Mimblewimble blockchain
implementeert en de lacunes vult die nodig zijn voor een
volledige blockchain en
cryptovaluta inzet

Het belangrijkste doel en eigenschappen van het Grin project zijn:

* Privacy als standaard. Dit maakt volledige fungibliteit mogelijk zonder
  het vermogen om selectief informatie vrij te geven indien nodig uit te sluiten.
* Schaalt meestal met het aantal gebruikers en minimaal met het aantal transacties
  (<100 byte `kernel), wat resulteert in een grotere ruimtebesparing
  vergeleken met andere blockchains.
* Sterk en bewezen cryptografie. Mimblewimble rust enkel op Elliptic Curve
  Cryptografie die al decennia beproefd en getest wordt.
* Eenvoud van het ontwerp die het makkelijk maakt om na verloop van tijd te
  controleren en onderhouden.
* Gemeenschapsgedreven, met behulp van een asic-resistant mining algoritme
  (Cuckoo Cycle) welke mining decentralisatie stimuleert.

## Betwisting voor Iedereen

Dit document is bedoeld voor lezers met een sterke achtergrond
van blockchains en elementaire cryptografie. Met dat in ons achterhoofd, proberen we
de technische opbouw van Mimblewimble en hoe het in Grin is toegepast uit te leggen.
We hopen dat dit document verstaanbaar is voor de meeste technische lezers
Ons doel is om u aan te moedigen geïnteresseerd te raken in Grin en
op welke manier mogelijk dan ook bij te dragen.

Om dit doel te bereiken, zullen we de belangrijkste concepten introduceren die vereist
zijn voor een goed begrip van Grin als een Mimblewimble-implementatie. We beginnen met een beknopte beschrijving
van enkele relevante eigenschappen van Elliptic Curve Cryptografie (ECC) om de basis
waarop Grin gebaseerd is en vervolgens alle belangrijke elementen van
Mimblewimble blockchain's transacties en -blokken te beschrijven.

### Minuscule Databits van Elliptic Curves

We beginnen met een korte inleiding van Elliptic Curve Cryptografie, waarbij we alleen
de eigenschappen evalueren die nodig zijn om om te begrijpen hoe
Mimblewimble werkt zonder te diep op de complexiteit van ECC in te gaan.
Voor lezer die dat wel zouden willen, zijn er andere mogelijkheden om
[er meer over te weten te komen](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

Een Elliptische Curve voor cryptografie is eenvoudig een groot aantal punten die we _C_ zullen noemen. De punten kunnen toegevoegd, afgetrokken of vermenigvuldigd worden met
gehele getallen (ook wel scalaire waarden genoemd).
Gegegeven een geheel getal _k_ en
met behulp van de scalaire vermenigvuldigingsoperatie kunnen we `k*H` berekenen, welke
ook een punt is op curve _C_.
Met een andere geheel getal _j_ kunnen we ook `(k+j)*H` berekenen, welke gelijk is
aan `k*H + j*H`. De toevoeging en scalaire vermenigvuldigingsoperaties op een
elliptische curve behoud de commutatieve en associatieve eigenschappen van optellen en vermenigvuldigen:

    (k+j)*H = k*H + j*H

Als we bij ECC een zeer groot getal _k_ als privésleutel kiezen, wordt `k*H`
beschouwd als de bijbehorende openbare sleutel. Zelfs als iemand de waarde van de
publieke sleutel `k*H`, het afleiden van _k_ is bijna onmogelijk (of anders gezegd,
terwijl vermenigvuldiging triviaal is, "verdeling" door curvepunten is extreem moeilijk).

De vorige formule `(k+j)*H = k*H + j*H`, met _k_ en _j_ als privésleutels,
demonstreert dat een openbare sleutel verkregen is door de toevoeging van
twee privésleutels (`(k+j)*H`) zijn identiek aan de toevoeging van de
openbare sleutels voor elk van die twee privésleutels (`k*H + j*H`).
In de Bitcoin blockchain, zijn Hiërarchische Deterministische portefeuilles
sterk afhankelijk van dit principe. Alsook Mimblewimble en de Grin-implementatie.

### Transacties met Mimblewimble

De structuur van transacties toont een cruciaal principe van Mimblewimble:
sterke privacy- en vertrouwelijkheidsgaranties.

De validatie van Mimblewimble transacties zijn gebaseerd op twee basiseigenschappen:

* **Verificatie van zero sums.** De som van de uitkomsten min de ingaven is altijd
  gelijk aan nul, welke bewijst dat de transactie geen nieuw geld gecreëerd heeft, _zonder de werkelijke bedragen te onthullen_.
* **Bezit van privésleutels.** Zoals bij de meeste andere cryptovaluta's, eigendom
  van transactieresultaten wordt gegarandeerd door het bezit van ECC privésleutels. Echter, het bewijs dat een entiteit die privésleutels bezit, wordt niet bereikt
  door rechtstreeks de transactie te ondertekenen.

De volgende secties over balans, eigendom, wisselbedrag en bewijzen beschrijven hoe
deze twee fundamentele eigenschappen bereikt worden.

#### Balans

Voortbouwend op de eigenschappen van ECC welke we hierboven beschreven hebben kan men
de waarden in een transactie verdoezelen.

Als _v_ de waarde is van een transactie-invoer of -uitvoer en _H_ een elliptische curve,
kunnen we eenvoudigweg `v*H` insluiten in plaats van _v_ bij een transactie. Dit werkt
omdat het gebruik maakt van de ECC operaties, we kunnen nog steeds de som valideren van
uitvoeren van een transactie welke gelijk is aan de som van inputs:

    v1 + v2 = v3  =>  v1*H + v2*H = v3*H

Door deze eigenschap op elke transactie te verifiëren,
kan het protocol verifiëren dat een transactie geen geld uit het niets creëert, zonder te weten wat de werkelijke waarden zijn.

Er zijn echter een eindig aantal bruikbare waarden en men zou elke waarde kunnen proberen om de waarde van uw transactie te raden.
one of them to guess the value of your transaction. Bovendien, de waarde van weten v1 (uit een vorige
transactie bijvoorbeeld) en de resulterende `v1*H` onthult alle uitkomsten met
waarde v1 over de blockchain. Om deze
redenen, introduceren we een tweede elliptische curve
_G_ (_G_ is praktisch gewoon een ander generatorspunt op dezelfde curvegroep als _H_) en
een privésleutel _r_ gebruikt als een *blinding factor*.

Een invoer- of uitvoerwaarde in een transactie kan vervolgens uitgedrukt worden als:

    r*G + v*H

Waar:

* _r_ een privésleutel is die gebruikt wordt als een blinding factor, _G_ is een elliptische curve en
  hun product `r*G` is de publieke sleutel voor _r_ on _G_.
* _v_ is de waarde van een invoer of uitvoer en _H_ is een andere elliptische curve.

Noch _v_ noch _r_ kan worden afgeleid, gebruikmakend van de fundamentele eigenschappen van Elliptische
Curve Cryptografie. `r*G + v*H` wordt een _Pedersen Commitment_ genoemd.

Laten we als voorbeeld aannemen dat we een transactie willen opbouwen met twee invoeren en één uitvoer.
We hebben (kosten genegeerd):

* vi1 en vi2 als invoerwaarden.
* vo3 als uitvoerwaarde.

Zodat:

    vi1 + vi2 = vo3

Een privésleutel genereren als blinding factor voor elke invoerwaarde en elke waarde vervangen
met hun respectievelijke Pedersen Commitments in de vorige vergelijken, verkrijgen we:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vo3*H)

Wat als gevolg vereist dat:

    ri1 + ri2 = ro3

Dit is de eerste pijler van Mimblewimble: de arithmetische vereist om een transactie te valideren gedaan kan worden
zonder dat één van de waarden gekend is.

Tot slot, is dit idee eigenlijk afgeleid van Greg Maxwell's
[Confidential Transactions](https://elementsproject.org/features/confidential-transactions/investigation),
welke zelf is afgeleid van een Adam Back voorstel voor homomorfische waarden toegepast
aan Bitcoin.

#### Eigendom

In het vorige gedeelte hebben we een privésleutel geïntroduceerd als een blinding factor om de transactiewaarden te verdoezelen.
Het tweede inzicht van Mimblewimble is dat deze privésleutel
gebruikt kan worden om het eigendom van de waarde aan te tonen.

Alice stuurt je 3 munten en om dat bedrag te verdoezelen, kies je 28 als jouw
blinding factor (constateer dat in de praktijk, de blinding factor een privésleutel is,
wat een extreem groot getal is). Ergens op de blockchain verschijnt de volgende uitvoer en
mag alleen door u besteed worden:

    X = 28*G + 3*H

_X_, het resultaat van de toevoeging, is zichtbaar voor iedereen. De waarde 3 is alleen bekend bij jou en Alice en 28 is alleen bekend bij u.

Om deze 3 munten opnieuw over te maken, vereist het protocol dat 28 op de een of andere manier bekend is.
Om aan te tonen hoe dit werkt, laten we zeggen
dat je die zelfde 3 munten wilt overmaken aan Carol.
Je een eenvoudige transactie moet bouwen, zodanig dat:

    Xi => Y

Waar _Xi_ een invoer is die je _X_ uitvoer doorgeeft en Y de uitvoer van Carol is. Er is geen manier om zulke transactie en balans te maken zonder uw privésleutel van 28 gekend is.
Inderdaad, als Carol deze transactie in evenwicht wilt brengen, moet zij zowel de verzonden waarde als uw privésleutel kennen, zodat:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H

Door na te gaan of alles het nulpunt bereikt heeft, kunnen we er opnieuw voor zorgen dat er geen nieuw geld gecreëerd is.

Wacht! Stop! Nu je de privésleutel in de uitvoer van Carol weet (welke in dit geval hezelfde moet zijn als de jouwe om evenwicht te brengen) en zo zou je het geld terug kunnen stelen van Carol!

Om dit op te lossen, gebruikt Carol een privésleutel naar keuze.
Ze kiest laten we zeggen: 113 en wat op de blockchain belandt is:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H

Nu de transactie niet meer op tot nul telt en we een _meerwaarde_ bij _G_
(85) hebben, welke het resultaat is van de sommatie van alle blinding factors. Maar doordat `85*G` een
geldige publieke sleutel is op de elliptische curve _C_, met privésleutel 85,
voor elke x en y, alleen als `y = 0` is `x*G + y*H` een geldige publieke sleutel op _G_.

Dus al wat het protocol nodig heeft is om te verifiëren dat (`Y - Xi`) is een geldige publieke sleutel op _G_ en dat
de partijen die de transactie aangaan, de privésleutel gezamenlijk kennen (85 in onze transactie met Carol). De
eenvoudigste manier om dit te doen, is door een handtekening te eisen die gemaakt is met de meerwaarde (85),
die vervolgens valideert dat:

* De partijen die de transactie aangaan, de privésleutel gezamenlijk kennen en
* De som van de transactie uitvoeren minus de invoeren, som tot een nulwaarde
  (omdat enkel een geldige publieke sleutel, overeenkomt met de privésleutel, de handtekening zal controleren).

Deze handtekening, gekoppeld aan elke transactie samen met enkele aanvullende gegevens (zoals
mining fees), wordt een _transaction kernel_ genoemd en wordt gecontroleerd door alle validateurs.

#### Wat details

Dit gedeelte gaat in op het maken van transacties door te bespreken hoe verandering geïntroduceerd is
en de vereiste voor range proofs zodat alle waarden bewezen zijn als niet-negatieve.
Geen van beide zijn absoluut vereist om Mimblewimble en
Grin te begrijpen, dus als je gehaast bent, voel je virj om meteen over te gaan naar
[Alles bij elkaar samenbrengen](#putting-it-all-together).

##### Wisselbedrag

Stel dat u slechts 2 munten wilt verzenden naar Carol van de 3 die u ontvangen heeft van Alice.
Om dit te doen zou je de resterende 1 munt naar jezelf terug moeten sturen als wisselgeld.
Je genereert een andere privésleutel (zeg 12) als een blinding factor om uw wisselgeld uitvoer te beschermen.
Carol gebruikt haar eigen privésleutel als voorheen.

    Wisselgeld uitvoer:     12*G + 1*H
    Carol's uitvoer:    113*G + 2*H

Wat op de blockchain terechtkomt is iets vergelijkbaars met voorheen.
En de handtekening is opnieuw gemaakt met de meerwaarde, 97 in dit voorbeeld.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H

##### Range Proofs

In alle bovenstaande berekeningen, vertrouwen we erop dat de transactiewaarden altijd positief zijn. De
invoering van negatieve bedragen zou uiterst problematisch zijn als men
nieuwe fondsen bij elke transactie zou kunnen maken.

Bijvoorbeeld, kan iemand een transactie creëren met een invoer van 2 en uitvoer van 5
en -3 en nog steeds een goed gebalanceerde transactie krijgen, volgens de definitie in de vorige secties. Dit kan niet makkelijk gedetecteerd worden, zelfs als _x_
negatief is, het overeenkomstige punt `x.H` op de curve lijkt op een ander.

Om dit te probleem te verhelpen, maakt Mimblewimble gebruik van een ander cryptografisch concept (ook afkomstig
vanuit Confidential Transactions) genaamd
range proofs: een bewijs dat een getal binnen een gegeven bereik valt, zonder het nummer te onthullen.
We gaan niet uitweiden op de range proof, maar u moet gewoon weten
dat voor elke `r.G + v.H` een bewijs gemaakt kan worden dat aan kan tonen dat _v_ groter is dan
nul en dat het niet uitloopt op een overflow.

Het is ook belangrijk om op te merken dat om een geldige range proof te maken uit bovenstaand voorbeeld, beide van de waarden 113 en 28 gebruikt zijn bij het maken en ondertekenen van de overschrijdingswaarde, bekend moeten zijn. De reden hiervoor, evenals een meer gedetailleerde beschrijving van range proof worden verder uitgelegd in de [range proof paper](https://eprint.iacr.org/2017/1066.pdf).

#### Alles bij elkaar samenbrengen

Een Mimblewimble transactie omvat het volgende:

* Een reeks invoeren, die verwijzen naar en een vorige reeks aan uitvoeren spendeert.
* Een reeks van nieuwe uitvoeren met:
  * Een waarde en een blinding factor (welke gewoonweg een nieuwe privésleutel is) vermenigvuldigd met
  een curve en bij elkaar opgeteld `r.G + v.H`.
  * Een range proof die aantoont dat v niet-negatief is..
* Een duidelijke expliciete transactiekost.
* Een handtekening, berekend door het overschrijden van de blinding waarde (de som van alle uitvoeren plus de kosten, min de invoeren) en gebruik het als een privésleutel.

### Blocks en Chain State

We hebben hierboven uitgelegd hoe Mimblewimble transacties sterke anonimiteit kunnen garanderen terwijl de eigenschappen die vereist zijn voor een geldige blockchain handhaaft,
d.w.z. een transactie creëert geen geld en een bewijs van eigendom wordt vastgelegd met privésleutels.

Het Mimblewimble blockformaat bouwt hierop voort door een aanvullend concept te introduceren: _cut-through_.
Met deze aanvulling, verkrijgt een Mimblewimble chain:

* Zeer goede schaalbaarheid, zoals de grote meerderheid van transactiegegevens geëlimineerd kunnen worden met de tijd,
zonder de beveiliging in gevaar te brengen.
* Verdere anonimiteit door het combineren en verwijderen van transactiegegevens.
* En de mogelijkheid voor nieuwe nodes om met de rest van het netwerk zeer efficiënt te synchroniseren.

#### Transaction Aggregatie

Niet vergeten dat een transactie uit het volgende bestaat -

* een reeks van invoeren die referen en een vorige reeks van uitvoeren spenderen
* een reeks nieuwe uitvoeren (Pedersen commitments)
* een transactiekernel, bestaande uit
  * kernel excess (Pedersen commitment naar nul)
  * transactiehandtekening (gebruikt kernel excess als publieke sleutel)

Een transactie is ondertekend en de handtekening is opgenomen in een _transaction kernel_. De handtekening wordt gegenereerd met behulp van de _kernel excess_ als een publieke sleutel die bewijst dat de transactie 0 bedraagt.

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

De publieke sleutel in dit voorbeeld is `28*G`.

We kunnen zeggen dat het volgende waar is voor elke geldige transactie (voor eenvoud worden kosten genegeerd) -

    som(uitvoeren) - som(invoeren) = kernel_excess

Hetzelfde geldt voor blokken zelf als we eenmaal realiseren dat een blok eenvoudigweg een reeks van geaggregeerde invoeren, uitvoeren en transactiekernels zijn.

    som(uitvoeren) - som(invoeren) = sum(kernel_excess)

Enigszins vereenvoudigd, (wederom negeren we de transactiekosten) kunnen we zeggen dat Mimblewimble-blokken behandeld kunnen worden als Mimblewimble-transacties.

##### Kernel Offsets

Er is een subtiel probleem met Mimblewimble-blokken en transacties zoals hierboven beschreven. Het is mogelijk (en in sommige gevallen triviaal) om de constituerende transacties in een blok te reconstrueren. Dit is duidelijk slecht voor privacy. Dit is een "subset" probleem - gegeven een verzameling van invoeren, uitvoeren en transactiekernels zal een subnet van dit formaat recombineren om een geldige transactie te reconstrueren.

Bijvoorbeeld, gegeven zijn de volgende twee transacties -

    (in1, in2) -> (uit1), (kern1)
    (in3) -> (uit2), (kern2)

We kunnen ze samenvoegen in de volgende blok (of geaggregeerde transactie) -

    (in1, in2, in3) -> (uit1, uit2), (kern1, kern2)

Het is triviaal eenvoudig om alle mogelijke permutaties uit te proberen om één van de transacties te recupereren (waar deze succeslvolg naar nul gesommeerd wordt) -

    (in1, in2) -> (uit1), (kern1)

We weten ook dat alles wat overblijft gebruikt kan worden om de andere geldige transactie te reconstrueren -

    (in3) -> (uit2), (kern2)

Om dit te beperken nemen we een _kernel offset_ op bij elke transactiekernel. Dit is een blinding factor (privésleutel) welke toegevoegd moet worden aan de kernel excess om de betrokken som naar nul te verifiëren -

    som(uitvoeren) - sum(invoeren) = kernel_excess + kernel_offset

Wanneer we transacties in een blok samenvoegen, bewaren we een _single_ geaggregeerde offset in de blockheader. Nu hebben we een single offset die niet ontbonden kan worden in de individuele transactie kernel offsets en de transacties kunnen niet meer worden gereconstrueerd -

    som(uitvoeren) - sum(invoeren) = sum(kernel_excess) + kernel_offset

We "splitsen" de sleutel `k` in `k1+k2` tijdens de transactie opbouw. Voor een transactie kernel `(k1+k2)*G` publiceren we `k1*G` (de overspil) en `k2` (de offset) en ondertekenen we de transactie met `k1*G` zoals eerder.
Tijdens de blockconstructie kunnen we eenvoudigweg de `k2` offsets optellen om een single aggregate `k2` offset te genereren om alle transacties in de blok te dekken. De `k2` offset voor elke individuele transactie is niet-terugvorderbaar.

#### Cut-through

Door blokken kunnen miners meerdere transacties samenstellen tot een enkele verzameling die toegevoegd wordt aan de chain.
In de volgende blokstappen, die 3 transacties bevat, tonen we alleen invoeren en uitvoeren van transacties.
Invoeren refereert uitvoeren die gespendeerd worden. Een uitvoer opgenomen in een vorige blok is gemarkeerd met een kleine x.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

We merken de volgende twee eigenschappen op:

* Binnen dit blok, worden sommige uitvoeren rechtstreeks uitgegeven door invoeren (I3
  besteedt O2 en I4 besteedt O3).
* De structuur van elke transactie doet er niet toe. Doordat alle transacties individueel sommeren op nul, de som van alle transactieinvoeren en -uitvoeren moet nul zijn.

Net als bij een transactie, is dat alles dat gecontroleerd moet worden in een blok is dat de eigendom bewezen is
(wat afkomstig is van _transaction kernels_) en dat de hele blok geen geldvoorraad heeft toegevoegd (anders dan wat toegestaan is door de coinbase).
Daarom kunnen overeenkomende invoeren en uitvoeren geëlimineerd worden, als hun bijdrage aan de totale som annuleert.
Welke leidt tot het volgende, een veel compactere blok:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Merk op dat alle transactiestructuren geëlimineerd zijn en de volgorde van invoeren en uitvoeren er niet meer to doet.
Echter, de som van alle uitvoeren in deze blok, min de invoeren, is nog steeds gegarandeerd nul.

Een blok is eenvoudigweg opgebouwd uit:

* Een blokhoofding.
* De lijst van invoeren die overblijven na de cut-through.
* De lijst van uitvoeren die overblijven na de cut-through.
* Een enkele kernel offset om de volledige blok te dekken.
* De transactiekernels bevatten voor elke transactie:
  * De publieke sleutel `r*G` verkregen uit de sommatie van alle geëngageerde.
  * De handtekeningen gegenereerd door middel van de overtollige waarde.
  * De miningkost.

Wanneer het op deze manier geconstructureerd wordt, biedt een Mimblewimble-blok buitengewoon goede privacygaranties:

* Intermediaire (cut-through) transacties worden alleen weergegeven door hun transactiekernels.
* Alle uitvoeren zien er hetzelfde uit: gewoon hele grote getallen die onmogelijk van elkaar te differentiëren zijn.
Als men sommige uitvoeren willen uitsluiten, dan moeten ze alles uitsluiten.
* Alle transactiestructuren zijn verwijderd, waardoor het onmogelijk is om te bepalen welke uitvoer gekoppeld werd aan elke invoer.

En toch, valideert dit alles nog steeds!

#### Cut-through All The Way

Nog even over het vorige voorbeeldblok, uitvoeren x1 en x2, uitgegeven door I1 en
I2, moet eerder verschenen zijn in de blockchain. Dus na de toevoeging van
deze blok, kunnen deze uitvoeren als I1 en I2 verwijderd worden van de
algehele chain, omdat ze niet kunnen bijdragen aan de totale som.

Veralgemeend, concluderen we dat de ketenstatus (met uitzondering van headers) op enig tijdstip
samengevat kunnen worden door alleen deze stukjes informatie:

1. Het totale aantal munten gecreëerd door mining in de keten.
1. De volledige verzameling aan niet-bestede uitgaven.
1. De transactiekernels voor elke transactie.

Het eerste stuk informatie kan afgeleid worden door alleen de blokhoogte
(de afstand tot het genesisblok) te gebruiken. En zowel de niet-bestede uitgaven als de
transactiekernels zijn ontzettend compact. Dit heeft 2 belangrijke gevolgen:

* De stand die een bepaalde node in een Mimblewimble blockchain moet behouden    blijven is zeer klein
  (in de volgorde van enkele gigabytes voor een bitcoin-sized blockchain en
  potentieel optimaliseerbaar tot enkele honderden megabytes).
* Wanneer een nieuwe node zich aansluit bij een netwerk ter bijdrage aan de Mimblewimble chain, is het aantal informatie die overdragen moet worden ook enorm klein.

Bovendien kan er niet met de gehele verzameling aan ongebruikte uitgaven gesjoemeld worden, zelfs
niet door een uitgave toe te voegen of te verwijderen. Daarmee zou de sommatie van alle
blinding factors in de transactiekernels om te verschillen van de sommatie van blinding
factors in de uitgaven.

### Conclusie

In dit document hebben we de basisprincipes behandeld die ten grondslag liggen van een Mimblewimble
blockchain. Door de aanvullende eigenschappen te gebruiken van Elliptic Curve Cryptografie, zijn
we in staat om transacties te bouwen die geheel ondoorzichtig zijn maar nog steeds goed gevalideerd kunnen worden.
En door deze eigenschappen te generaliseren naar blokken, kunnen we een grote hoeveelheid aan blockchaingegevens elimineren,
waardoor grote schalering en snelle synchronisatie
van nieuwe peers mogelijk is.
