# Introduktion till Mimblewimble och Grin

*Läs detta på andra språk: [English](../intro.md), [Español](intro_ES.md), [Nederlands](intro_NL.md), [Русский](intro_RU.md), [日本語](intro_JP.md), [Deutsch](intro_DE.md), [Portuguese](intro_PT-BR.md), [Korean](intro_KR.md), [简体中文](intro_ZH-CN.md).*

Mimblewimble är ett blockkedjeformat och protokoll som erbjuder extremt bra
skalbarhet, integritet, och fungibilitet genom starka kryptografiska primitiver.
Den angriper brister som existerar i nästan alla nuvarande blockkedjeimplementationer.

Grin är ett mjukvaruprojekt med öppen källkod som implementerar en Mimblewimble-blockkedja
och fyller igen luckorna för att skapa en fullständig blockkedja och kryptovaluta.

Grin-projektets huvudsakliga mål och kännetecken är:

* Integritet som standard. Detta möjliggör fullkomlig fungibilitet utan att
förhindra förmågan att selektivt uppdaga information efter behov.
* Växer mestadels med antal användare och minimalt med antal transaktioner (< 100 bytes transaktionskärna),
vilket resulterar i stora utrymmesbesparingar i jämförelse med andra blockkedjor.
* Stark och bevisad kryptografi. Mimblewimble förlitar sig endast på kryptografi med
elliptiska kurvor (ECC) vilket har beprövats i decennier.
* Simplistik design som gör det enkelt att granska och underhålla på lång sikt.
* Gemenskapsdriven, uppmuntrar mining och decentralisering.

## Tungknytande för alla

Detta dokument är riktat mot läsare med en bra förståelse för blockkedjor och grundläggande kryptografi.
Med det i åtanke försöker vi förklara den tekniska uppbyggnaden av Mimblewimble och hur det appliceras i Grin.
Vi hoppas att detta dokument är föreståeligt för de flesta tekniskt inriktade läsare. Vårt mål är att
uppmuntra er att bli intresserade i Grin och bidra på något möjligt sätt.

För att uppnå detta mål kommer vi att introducera de huvudsakliga begrepp som krävs för en
bra förståelse för Grin som en  Mimblewimble-implementation. Vi kommer att börja med en kort
beskrivning av några av elliptiska kurvornas relevanta egenskaper för att lägga grunden som Grin
är baserat på och därefter beskriva alla viktiga element i en Mimblewimble-blockkedjas
transaktioner och block.

### Småbitar av elliptiska kurvor

Vi börjar med en kort undervisning i kryptografi med elliptiska kurvor (ECC) där vi endast
går igenom de nödvändiga egenskaper för att förstå hur Mimblewimble fungerar utan att
gå djupt in på dess krångligheter. För läsare som vill fördjupa sig i detta finns andra
möjligheter att [lära sig mer](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

En elliptisk kurva för kryptografiska är ändamål är enkelt sagt en stor mängd av punkter
som vi kallar för _C_. Dessa punkter kan adderas, subtraheras, eller multipliceras med heltal (även kallat skalärer).
Given en sådan punkt _H_ och ett heltal _k_ kan vi beräkna `k*H` med skalärmultiplikation, vilket också är en punkt på kurvan _C_. Given ett annat
heltal _j_ kan vi också beräkna `(k+j)*H`, vilket är lika med `k*H + j*H`. Addition och skalärmultiplikation på elliptiska
kurvor behåller sina kommutativa och associativa egenskaper från vanlig addition och multiplikation:

    (k+j)*H = k*H + j*H

Om vi inom ECC väljer ett väldigt stort tal _k_ som privat nyckel så anses `k*H` vara dess publika nyckel. Även om
man vet värdet av den publika nyckeln `k*H`, är det nästintill omöjligt att härleda `k` (sagt med andra ord, medan
multiplikation med kurvpunkter är trivialt är "division" extremt svårt).

Den föregående formeln `(k+j)*H = k*H + j*H`, med _k_ och _j_ båda som privata nycklar, demonstrerar att en publik nyckel
erhållen av att ha adderat de två privata nycklarna är identisk med de två privata nycklarnas respektive
publika nycklar adderade (`k*H + j*H`). I Bitcoin-blockkedjan använder hierarkiska deterministiska plånböcker (HD wallets)
sig flitigt av denna princip. Mimblewimble och Grin-implementationer gör det också.

### Transaktioner med Mimblewimble

Transaktionernas struktur demonstrerar en av Mimblewimbles kritiska grundsatser:
starka garantier av integritet och konfidentialitet.

Valideringen av Mimblewimble-transaktioner använder sig av två grundläggande egenskaper:

* **Kontroll av nollsummor.** Summan av outputs minus inputs är alltid lika med noll, vilket bevisar—utan att
avslöja beloppen—att transaktionen inte skapade nya pengar.
* **Innehav av privata nycklar.** Som med de flesta andra kryptovalutor garanteras ägandet av outputs (UTXOs)
med innehavet av privata nycklar. Dock bevisas inte ägandet av dem genom en direkt signering av transaktionen.

De följande styckena angående saldo, ägande, växel, och range proofs klarlägger hur de två grundläggande egenskaperna uppnås.

#### Saldo

Bygger vi på ECC-egenskaperna vi förklarade ovan kan vi beslöja beloppen i en transaktion.

Om _v_ är beloppet av en input eller output och _H_ en punkt på den elliptiska kurvan _C_, kan vi enkelt bädda in
`v*H` i stället för _v_ i en transaktion. Detta fungerar eftersom vi fortfarande kan bekräfta att summan av outputs är
lika med summan av inputs i en transaktion med hjälp av ECC-operationer:

    v1 + v2 = v3 => v1*H + v2*H = v3*H

Bekräftandet av denna egenskap på alla transaktioner låter protokollet bekräfta att en transaktion inte skapar pengar ur
tomma intet utan att veta vad beloppen är. Dock finns det ett begränsat antal av användbara belopp och man skulle kunna
prova varenda en för att gissa beloppet på transaktionen. Dessutom, om man känner till _v1_ (till exempel från en föregående
transaktion) och det resulterande `v1*H` avslöjas alla outputs med beloppet _v1_ över hela blockkedjan. Av dessa
anledningar introducerar vi en andra punkt _G_ på samma elliptiska kurva och en privat nyckel _r_ som används som en *förblindningsfaktor*.

En input eller output i en transaktion kan uttryckas som:

    r*G + v*H

Där:

* _r_ är en privat nyckel använd som en förblindningsfaktor, _G_ är en punkt på elliptiska kurvan _C_, och deras
produkt `r*G` är den publika nyckeln för _r_ (med _G_ som generatorpunkt).
* _v_ är ett input- eller output-belopp och _H_ är en annan punkt på kurvan _C_ som tillsammans producerar en annan
public nyckel `v*H` (med _H_ som generatorpunkt).

Varken _v_ eller _r_ kan härledas på grund av ECC:s grundläggande egenskaper. `r*G + v*H` kallas för
ett _Pedersen Commitment_.

Som ett exempel, låt oss anta att vi vill skapa en transaktion med två inputs och en output.
Vi har (utan hänsyn till avgifter):

* vi1 och vi2 som input-belopp.
* vo3 som output-belopp.

Sådana att:

    vi1 + vi2 = vo3

Vi genererar en privat nyckel som en förblidningsfaktor för varje input och ersätter alla belopp med
deras respektive Pedersen Commitment och får därmed:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vi3*H)

Vilket som följd kräver att:

    ri1 + ri2 = ro3

Detta är Mimblewimbles första pelare: de beräkningar som är nödvändiga för att validera en transaktion
kan göras utan att veta några belopp.

Denna idé härstammar faktiskt från Greg Maxwells
[Confidential Transactions](https://elementsproject.org/features/confidential-transactions/investigation),
som i sin tur härstammar från ett förslag av Adam Back för homomorfiska belopp applicerade på Bitcoin.

#### Ägande

I föregående stycke introducerade vi en privat nyckel som en förblindningsfaktor för att dölja transaktionens belopp.
Mimblewimbles andra insikt är att denna privata nyckel kan användas för att bevisa ägande av beloppet.

Alice skickar 3 mynt till dig och för att dölja beloppet väljer du 28 som din förblindningsfaktor (notera att förblindningsfaktorn i praktiken
är ett extremt stort tal). Någonstans i blockkedjan dyker följande output upp och ska endast kunna spenderas av dig:

    X = 28*G + 3*H

_X_ som är summan är synlig för alla. Beloppet 3 är endast känt av dig och Alice, och 28 är endast
känt av dig.

För att skicka dessa 3 mynt igen kräver protokollet att 28 ska vara känt. För att demonstrera hur detta fungerar, låt
oss säga att du vill skicka samma 3 mynt till Carol. Du behöver skapa en simpel transaktion sådan att:

    Xi => Y

Där _Xi_ är en input som spenderar din _X_-output och Y är Carols output. Det finns inget sätt att skapa
en sådan transaktion utan att känna till din privata nyckel 28. Om Carol ska balansera denna transaktion behöver hon
både känna till det skickade beloppet och din privata nyckel så att:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H

Genom att kontrollera att allt har nollställts kan vi återigen försäkra oss om att inga nya pengar har skapats.

Vänta! Stopp! Nu känner du till den privata nyckeln i Carols output (vilket i detta fall måste vara samma som ditt
för att balansera inputs och outputs) så du skulle kunna stjäla tillbaka pengarna från Carol!

För att lösa detta problem använder Carol en privat nyckel som hon väljer själv. Låt oss säga att hon väljer 113.
Det som hamnar i blockkedjan är:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H

Nu summeras transaktionen inte längre till noll och vi har ett _överskottsbelopp_ (85), vilket är resultatet
av summeringen av alla förblindningsfaktorer. Eftersom `85*G` är en giltig publik nyckel för generatorpunkt _G_ vet vi
att alla inputs och outputs har balanserats och transaktionen därmed är giltig då `x*G + y*H` är en giltig publik nyckel för generatorpunkt _G_ om och endast om `y = 0`.

Så allt protokollet behöver göra är att kontrollera att (`Y - Xi`) är en giltig publik nyckel för generatorpunkt _G_ och att de två parter
som utför transaktionen tillsammans kan producera dess privata nyckel (85 i exemplet ovan). Det enklaste sättet att göra
det är att kräva en signatur med överskottsbeloppet (85), vilket bekräftar att:

* De parter som utför transaktionen tillsammans kan beräkna den privata nyckeln (överskottsbeloppet)
* Summan av outputs minus inputs i transaktionen är noll (eftersom endast en giltig publik nyckel kan validera signaturen).

Denna signatur som tillsammans med lite annan information (som exempelvis mining-avgifter) bifogas till transaktionen kallas
för _transaktionskärna_ och kontrolleras av alla validerare.

#### Några finare punkter

Detta stycke detaljerar byggandet av transaktioner genom att diskutera hur växel införs och kravet för "range proofs"
så att alla belopp är bevisade att vara icke-negativa. Inget av detta är absolut nödvändigt för att förstå Mimblewimble
och Grin, så om du har bråttom känn dig fri att hoppa direkt till [Sammanställningen av allt](#sammanställningen-av-allt).

#### Växel

Låt oss säga att du endast vill skicka 2 mynt till Carol av de 3 mynt du mottog från Alice. För att göra detta behöver du
skicka det återstående myntet tillbaka till dig själv som växel. Du genererar en annan privat nyckel (t ex 12) som en
förblindningsfaktor för att skydda beloppet på din växel-output. Carol använder sin egen privata nyckel som tidigare.

    Växel-output:   12*G + 1*H
    Carols output:  113*G + 2*H

Det som hamnar i blockkedjan är något väldigt likt det vi hade tidigare, och signaturen är återigen skapat med
överskottsbeloppet, 97 i detta exempel.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H

#### Range Proofs

I alla beräkningar ovan förlitar vi oss på att alla belopp är positiva. Introduktionen av negativa belopp skulle vara
extremt problematiskt då man skulle kunna skapa nya pengar i varje transaktion.

Till exempel skulle man kunna skapa en transaktion med input-belopp 2 och output-belopp 5 och -3 och fortfarande
ha en balanserad transaktion. Detta kan inte upptäcklas enkelt eftersom punkten `x*H` ser ut som vilken annan punkt
som helst på kurvan även om _x_ är negativt.

För att lösa detta problem använder Mimblewimble sig av ett kryptografiskt koncept som kallas "range proofs" (som också härstammar
från Confidential Transactions): ett bevis på att ett tal befinner sig inom ett visst intervall utan att avsölja talet.
Vi kommer inte att förklara range proofs; du behöver endast veta att vi för varje `r*G + v*H` kan skapa ett bevis som visar
att _v_ är större än noll och inte orsakar overflow.

Det är även viktigt att notera att range proofs krävs för både förblindningsfaktorn och beloppet. Anledningen till detta är att det förhindrar en censureringsattack där en tredje part skulle kunna låsa en UTXO utan att känna till desss privata nyckel genom att skapa följande transaktion:

    Carols UTXO:          133*G + 2*H
    Attackerarens output: (113 + 99)*G + 2*H

vilket kan signeras av attackeraren eftersom Carols förblindningsfaktor nollställs i ekvationen `Y - Xi`:

    Y - Xi = ((113 + 99)*G + 2*H) - (113*G + 2*H) = 99*G

Denna output (`(113 + 99)*G + 2*H`) kräver att både talen 113 och 99 är kända för att kunna spenderas; attackeraren skulle därmed ha lyckats låsa Carols UTXO. Kravet på range proof för förblindingsfaktorn förhindrar detta eftersom attackeraren inte känner till 113 och därmed inte heller (113 + 99). En mer utförlig beskrivning av range proofs är förklarat i
[range proof-pappret](https://eprint.iacr.org/2017/1066.pdf).

#### Sammanställningen av allt

En Mimblewimble-transaktion inkluderar följande:

* En mängd inputs som refererar till och spenderar en mängd föregående outputs.
* En mängd nya outputs som inkluderar:
  * Ett belopp och en förblindningsfaktor (vilket bara är en ny privat nyckel) multiplicerade på en kurva och adderade
  till att bli `r*G + v*H`.
  * Ett range proof som bland annat visar att v är icke-negativt.
* En transaktionsavgift i klartext.
* En signatur vars privata nyckel beräknas genom att ta överskottsbeloppet (summan av alla outputs och
avgiften minus inputs).

### Block och kedjetillstånd

Vi förklarade ovan hur Mimblewimble-transaktioner kan erbjuda starka anonymitetsgarantier samtidigt som de
upprätthåller egenskaperna för en giltig blockkedja, d v s att en transaktion inte skapar pengar och att ägandebevis
fastställs med privata nycklar.

Mimblewimble-blockformatet bygger på detta genom att introducera ett till koncept: _cut-through_. Med detta
får en Mimblewimble-kedja:

* Extremt bra skalbarhet då den stora majoriteten av transaktionsinformation kan elimineras på lång sikt utan att
kompromissa säkerhet.
* Ytterligare anonymitet genom att blanda och ta bort transaktionsinformation.

#### Transaktionsaggregation

Kom igåg att en transaktion består av följande:

* En mängd inputs som refererar till och spenderar en mängd föregående outputs
* En mängd nya outputs
* En transaktionskärna som består av:
  * kärnöverskottet (överskottsbeloppets publika nyckel)
  * transaktionssignatur vars publika nyckel är kärnöverskottet

En transaktion valideras genom att kärnöverskottet faställs vara en giltig publik nyckel:

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

Den publika nyckeln i detta exempel är `28*G`.

Vi kan säga att följande är sant för alla giltiga transaktioner (vi ignorerar avgifter för enkelhetens skull):

    (summan av outputs) - (summan av inputs) = kärnöverskott

Detsamma gäller för blocken själva när vi inser att ett block helt enkelt är en mängd aggregerade inputs, outputs, och
transaktionskärnor. Vi kan summera alla outputs, subtrahera det med summan av alla inputs, och likställa vårt resulterande Pedersen commitment med summan av kärnöverskotten:

    (summan av outputs) - (summan av inputs) = (summan av kärnöverskott)


Något förenklat (återigen utan hänsyn till transaktionsavgifter) kan vi säga att Mimblewimble-block kan betraktas precis som
Mimblewimble-transaktioner.

##### Kärn-offset

Det finns ett subtilt problem med Mimblewimble-block och transaktioner som beskrivet ovan. Det är möjligt (och i vissa fall
trivialt) att rekonstruera de konstituerande transaktionerna i ett block. Detta är naturligtvis dåligt för integriteten.
Detta kallas för "delmängdsproblemet": givet en mängd inputs, outputs, och transaktionskärnor kommer någon delmängd av detta
kunna kombineras för att rekonstruera en giltig transaktion.

Betrakta dessa två transaktioner:

    (input1, input2) -> (output1), (kärna1)
    (input3) -> (output2), (kärna2)

Vi kan aggregera dem till följande block:

    (input1, input2, input3) -> (output1, output2), (kärna1, kärna2)

Det är trivialt att testa alla möjliga kombinationer och återskapa en av transaktionerna (där summan lyckas bli noll).

    (input1, input2) -> (output1), (kärna1)

Vi vet också att allt som kvarstår kan användas för att rekonstruera den andra giltiga transaktionen:

    (input3) -> (output2), (kärna2)

Kom ihåg att kärnöverskottet `r*G` helt enkelt är den publika nyckeln till överskottsbeloppet *r*. För att lösa detta problem omdefinierar vi kärnöverskottet från `r*G` till `(r-kärn_offset)*G` och distribuerar detta *kärn-offset* för att inkluderas i varje transktionskärna. Detta kärn-offset är således en förblindningsfaktor som måste tilläggas överskottsbeloopet för att ekvationen ska gå ihop:

    (summan av outputs) - (summan av inputs) = r*G = (r-kärn_offset)*G + kärn_offset*G

eller alternativt

    (summan av outputs) - (summan av inputs) = kärnöverskott + kärn_offset*G

För ett commitment `r*G + 0*H` med kärn-offset *a*, signeras transaktionen med `(r-a)` och *a* publiceras så att `r*G` kan beräknas för att kontrollera att transaktionen är giltig. Vid block-konstruktionen summeras alla kärn-offsets till ett enstaka sammanlagt offset som täcker hela blocket. Kärn-offsetet för en individuell transaktion blir därmed omöjlig att härleda och delmängdsproblemet är löst.

    (summan av outputs) - (summan av inputs) = (summan av kärnöverskott) + kärn_offset*G

#### Genomskärning

Blocks låter miners sätta ihop flera transaktioner till en enstaka struktur som läggs till på kedjan. I följande
block-representationer med tre transaktioner visar vi endast inputs och outputs. Inputs refererar till
föregående outputs som härmed spenderas. Föregående outputs markeras med _x_.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

Vi lägger märke till följande två egenskaper:

* Inom detta block är vissa outputs spenderade direkt av påföljande inputs (I3 spenderar O2, och I4 spenderar O3).
* Transaktionernas struktur spelar faktiskt ingen roll. Eftersom alla transaktioner individuellt summeras till noll
måste summan av alla inputs och outputs också vara noll.

Liknande en transaktion, är allt som behöver kontrolleras i ett block ägandebevis (vilket kommer från transaktionskärnorna)
och att blocket i helhet inte skapade pengar ur tomma intet (förutom det som är tillåtet vid mining). Således kan matchande inputs och outputs elimineras, då
deras sammansatta påverkan är noll. Detta leder till följande, mycket mer kompakta block:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Notera att all transaktionsstruktur har eliminerats och att ordningen av inputs och outputs inte längre spelar någon roll.
Summan av alla inputs och outputs är garanterat fortfarande noll.

Ett block består av:

* En block header.
* En lista av alla inputs som kvarstår efter genomskärning.
* En lista av alla outputs som kvarstår efter genomskärning.
* Ett enstaka kärn-offset (aggregatet av alla kärn-offset) som skyddar hela blocket.
* Transaktionskärnor för varje transaktion som innehåller:
  * Publika nyckeln `r*G` erhållen genom summation av alla inputs och outputs.
  * Signaturen genererad av överskottsbeloppet.
  * Mining-avgiften

Med denna struktur erbjuder ett Mimblewimble-block extremt bra integritetsgarantier:

* Mellanliggande (genomskurna) transaktioner är endast representerade av sina transaktionskärnor.
* Alla outputs ser likadana ut: väldigt stora tal som inte går att skilja åt på något meningsfullt sätt.
Om någon skulle vilja exkludera en specifik output skulle de vara tvungna att exkludera alla.
* All transaktionsstruktur har tagits bort vilket gör det omöjligt att se vilka inputs och outputs som passar ihop.

Men ändå kan allting valideras!

#### Genomskärning hela vägen

Vi går tillbaka till blocket i föregående exempel. Outputs x1 och x2 som spenderades av I1 och I2 måste ha
dykt upp tidigare i blockkedjan. Efter att detta block adderas till blockkedjan kan dessa outputs tillsammans med
I1 och I2 alla tas bort från blockkedjan eftersom de nu är mellanliggande transaktioner.

Vi slutleder att kedjetillståndet kan (bortsett från block headers) vid varje tidspunkt sammanfattas med endast dessa tre ting:

1. Den totala mängden mynt skapade genom mining.
1. Den kompletta mängden av UTXOs.
1. Transaktionskärnorna för varje transaktion.

Det första kan härledas genom att endast observera blockhöjden.

Både mängden av UTXOs och transaktionskärnorna är extremt kompakta. Detta har två följder:

* En nod i en Mimblewimble-blockkedja får en väldigt liten kedja att behöva ta vara på.
* När en ny nod ansluter sig till nätverket krävs det väldigt lite information för att den ska bygga kedjan.

Dessutom kan man inte manipulera mängden av UTXOs. Tar man bort ett element ändras summan av transaktionerna och är längre inte lika med noll.

### Slutsats

I detta dokument gick vi igenom de grundläggande principerna för en Mimblewimble-blockkedja. Genom att använda egenskaperna
för addition i kryptografi med elliptiska kurvor kan vi skapa fullständigt förmörkade transaktioner som ändå kan valideras.
Genom att generalisera dessa egenskaper till block kan vi eliminera en stor mängd blockkedjeinformation vilket medför
väldigt bra skalbarhet.
