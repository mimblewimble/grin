# Introduktion till MimbleWimble och Grin

*Läs detta på andra språk: [English](intro.md), [简体中文](intro_ZH-CN.md), [Español](intro_ES.md), [Nederlands](intro_NL.md), [Русский](intro_RU.md), [日本語](intro_JP.md), [Deutsch](intro_DE.md).*

MimbleWimble är ett blockkedjeformat och protokoll som erbjuder extremt bra
skalbarhet, integritet, och fungibilitet genom starka kryptografiska primitiver.
Den angriper brister som existerar i nästan alla nuvarande blockkedjeimplementationer.

Grin är ett mjukvaruprojekt med öppen källkod som implementerar en MimbleWimble-blockkedja
och fyller igen luckorna för att skapa en fullständig blockkedja och kryptovaluta.

Grin-projektets huvudsakliga mål och kännetecken är:

* Integritet som standard. Detta möjliggör fullkomlig fungibilitet utan att
förhindra förmågan att selektivt uppdaga information efter behov.
* Växer mestadels med antal användare och minimalt med antal transaktioner (< 100 bytes transaktionskärna), 
vilket resulterar i stora utrymmesbesparingar i jämförelse med andra blockkedjor.
* Stark och bevisad kryptografi. MimbleWimble förlitar sig endast på kryptografi med 
elliptiska kurvor (ECC) vilket har beprövats i decennier.
* Simplistik design som gör det enkelt att granska och underhålla på lång sikt.
* Gemenskapsdriven, uppmuntrar mining och decentralisering.

## Tungknytande för alla

Detta dokument är riktat mot läsare med en bra förståelse för blockkedjor och grundläggande kryptografi.
Med det i åtanke försöker vi förklara den tekniska uppbyggnaden av MimbleWimble och hur det appliceras i Grin.
Vi hoppas att detta dokument är föreståeligt för de flesta tekniskt inriktade läsare. Vårt mål är att
uppmuntra er att bli intresserade i Grin och bidra på något möjligt sätt.

För att uppnå detta mål kommer vi att introducera de huvudsakliga begrepp som krävs för en
bra förståelse för Grin som en  MimbleWimble-implementation. Vi kommer att börja med en kort
beskrivning av några av elliptiska kurvornas relevanta egenskaper för att lägga grunden som Grin
är baserat på och därefter beskriva alla viktiga element i en MimbleWimble-blockkedjas
transaktioner och block.

### Småbitar av elliptiska kurvor

Vi börjar med en kort undervisning i kryptografi med elliptiska kurvor (ECC) där vi endast
går igenom de nödvändiga egenskaper för att förstå hur MimbleWimble fungerar utan att
gå djupt in på dess krångligheter. För läsare som vill fördjupa sig i detta finns andra
möjligheter att [lära sig mer](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

En elliptisk kurva för kryptografiska är ändamål är enkelt sagt en stor mängd av punkter
som vi kallar för _C_. Dessa punkter kan adderas, subtraheras, eller multipliceras med heltal (även kallat skalärer).
Given ett heltal _k_ kan vi beräkna `k*H` med skalärmultiplikation, vilket också är en punkt på kurvan _C_. Given ett annat
heltal _j_ kan vi också beräkna `(k+j)*H`, vilket är lika med `k*H + j*H`. Addition och skalärmultiplikation på elliptiska
kurvor behåller sina kommutativa och associativa egenskaper från vanlig addition och multiplikation:

    (k+j)*H = k*H + j*H
    
Inom ECC, om vi väljer ett väldigt stort tal _k_ som privat nyckel så anses `k*H` vara dess publika nyckel. Även om
man vet värdet av den publika nyckeln `k*H`, är det nästintill omöjligt att härleda `k` (sagt med andra ord, medan
multiplikation är trivialt är "division" med kurvpunkter extremt svårt).

Den föregående formeln `(k+j)*H = k*H + j*H`, med _k_ och _j_ båda privata nycklar demonstrerar att en publik nyckel
erhållen av att ha adderat de två privata nycklarna är identisk med de två privata nycklarnas respektive
publika nycklar adderade (`k*H + j*H`). I Bitcoin-blockkedjan använder hierarkiska deterministiska plånböcker (HD wallets)
sig flitigt av denna princip. MimbleWimble och Grin-implementationer gör det också.

### Transaktioner med MimbleWimble

Transaktionernas struktur demonstrerar en av MimbleWimbles kritiska grundsatser:
starka garantier av integritet och konfidentialitet.

Valideringen av MimbleWimble-transaktioner använder sig av två grundläggande egenskaper:

* **Kontroll av nollsummor.** Summan av utmatningar minus inmatningar är alltid lika med noll, vilket bevisar—utan att 
avslöja beloppen—att transaktionen inte skapade nya pengar.
* **Innehav av privata nycklar.** Som med de flesta andra kryptovalutar garanteras ägandet av transaktionsutmatningar
med innehavet av privata nycklar. Dock bevisas inte ägandet av dem genom en direkt signering av transaktionen.

De följande styckena angående saldo, ägande, växel, och bevis klarlägger hur de två grundläggande egenskaperna uppnås.

#### Saldo

Bygger vi på ECC-egenskaperna vi förklarade ovan kan vi beslöja beloppen i en transaktion.

Om _v_ är beloppet av en inmatning eller utmatning i en transaktion och _H_ en elliptisk kurva, kan vi enkelt bädda in
`v*H` i stället för _v_ i en transaktion. Detta fungerar eftersom vi fortfarande kan bekräfta att summan av utmatningarna är
lika med summan av inmatningarna i en transaktion med hjälp av ECC-operationer:

    v1 + v2 = v3 => v1*H + v2*H = v3*H
    
Bekräftandet av denna egenskap på alla transaktioner låter protokollet bekräfta att en transaktion inte skapar pengar ur
tomma intet utan att veta vad beloppen är. Dock finns det ett begränsat antal av användbara belopp och man skulle kunna
prova varenda en för att gissa beloppet på din transaktion. Dessutom, om man känner till v1 (till exempel från en föregående
transaktion) och det resulterande `v1*H` avslöjar man alla utmatningar med beloppet v1 över hela blockkedjan. Av dessa
anledningar introducerar vi en till elliptisk kurva _G_ (i praktiken är _G_ endast en annan generatorpunkt på samma kurvgrupp
som _H_) och en privat nyckel _r_ som används som en *bländande faktor*.

Ett inmatnings- eller utmatningsbelopp i en transaktion kan uttryckas som:

    r*G + v*H
    
Där:

* _r_ är en privat nyckel använd som en bländande faktor, _G_ är en elliptisk kurva, och deras
produkt `r*G` är den publika nyckeln för _r_ på _G_.
* _v_ är ett inmatnings- eller utmatningsbelopp och _H_ är en annan elliptisk kurva.

Varken _v_ eller _r_ kan härledas på grund av ECC:s grundläggande egenskaper. `r*G + v*H` kallas för
ett _Pedersen Commitment_.

Som ett exempel, låt oss anta att vi vill skapa en transaktion med två inmatningar och en utmatning.
Vi har (utan hänsyn till avgifter):

* vi1 och vi2 som inmatningsbelopp.
* vo3 som utmatningsbelopp.

Sådana att:

    vi1 + vi2 = vo3
    
Vi genererar en privat nyckel som en bländande faktor för varje inmatningsbelopp och ersätter alla belopp med
deras respektive Pedersen Commitment och ekvationen blir därmed:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vi3*H)
    
Vilket som följd kräver att:

    ri1 + ri2 = ro3
    
Detta är MimbleWimbles första pelare: de beräkningar som är nödvändiga för att validera en transaktion
kan göras utan att veta några belopp.

Denna idé härstammar faktiskt från Greg Maxwells
[Confidential Transactions](https://elementsproject.org/features/confidential-transactions/investigation),
som i sin tur härstammar från ett förslag av Adam Back för homomorfiska belopp applicerade på Bitcoin.

#### Ägande

I föregående stycke introducerade vi en privat nyckel som en bländande faktor för att dölja transaktionens belopp.
MimbleWimbles andra insikt är att denna privata nyckel kan användas för att bevisa ägande av beloppet.

Alice skickar 3 mynt till dig och för att dölja beloppet väljer du 28 som din bländande faktor (notera att i praktiken
är den bländande faktorn ett extremt stort tal). Någonstans i blockkedjan dyker följande utmatning upp och ska endast 
vara spenderbar av dig:

    X = 28*G + 3*H
    
_X_ som är resultatet av additionen är synlig för alla. Beloppet 3 är endast känt av dig och Alice, och 28 är endast
känt av dig.

För att skicka dessa 3 mynt igen kräver protokollet att 28 ska vara känt. För att demonstrera hur detta fungerar, låt
oss säga att du vill skicka samma 3 mynt till Carol. Du behöver skapa en simpel transaktion sådan att:

    Xi => Y
    
Där _Xi_ är en inmatning som spenderar din _X_-utmatning och Y är Carols utmatning. Det finns inget sätt att skapa
en sådan transaktion utan att känna till din privata nyckel 28. Om Carol ska balansera denna transaktion behöver hon
både känna till det skickade beloppet och din privata nyckel så att:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H
    
Genom att kontrollera att allt har nollställts kan vi återigen försäkra oss om att inga nya pengar har skapats.

Vänta! Stopp! Nu känner du till den privata nyckeln i Carols utmatning (vilket i detta fall måste vara samma som ditt
för att balansera in- och utmatningarna) så du skulle kunna stjäla tillbaka pengarna från Carol!

För att lösa detta problem använder Carol en privat nyckel som hon väljer själv. Låt oss säga att hon väljer 113.
Det som hamnar i blockkedjan är:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H
    
Nu summeras transaktionen inte längre till noll och vi har ett _överskottsbelopp_ på _G_ (85), vilket är resultatet
av summeringen av alla bländande faktorer. Men eftersom `85*G` är en giltig publik nyckel på elliptiska kurvan _G_ vet vi
att in- och utmatningarna har subtraheras till noll och transaktionen är därmed giltig.

Så allt protokollet behöver göra är att kontrollera att (`Y - Xi`) är en giltig publik nyckel på _G_ och att de två parter
som utför transaktionen tillsammans kan producera den privata nyckeln (85 i exemplet ovan). Det enklaste sättet att göra
det är att kräva en signatur med överskottsbeloppet (85), vilket bekräftar att:

* De parter som utför transaktionen känner till den privata nyckeln, och
* Summan av utmatningarna minus inmatningarna i transaktionen är noll (eftersom överskottsbeloppet måste vara en publik nyckel).

Denna signatur som tillsammans med lite annan information (som exempelvis mining-avgifter) bifogas till transaktionen kallas
för _transaktionskärna_ och kontrolleras av alla validerare.

#### Några finare punkter

Detta stycke detaljerar byggandet av transaktioner genom att diskutera hur växel införs och kravet för "range proofs"
så att alla belopp är bevisade att vara icke-negativa. Inget av detta är absolut nödvändigt för att förstå MimbleWimble
och Grin, så om du har bråttom känn dig fri att hoppa direkt till [Sammanställningen av allt](#sammanställningen-av-allt).

#### Växel

Låt oss säga att du endast vill skicka 2 mynt till Carol av de 3 mynt du mottog från Alice. För att göra detta behöver du
skicka det återstående myntet tillbaka till dig själv som växel. Du genererar en annan privat nyckel (t ex 12) som en
bländande faktor för att skydda ditt växel-utmatningsbelopp. Carol använder sin egen privata nyckel som tidigare.

    Växel-utmatning:   12*G + 1*H
    Carols utmatning:  113*G + 2*H
    
Det som hamnar i blockkedjan är något väldigt likt det vi hade tidigare, och signaturen är återigen skapat med
överskottsbeloppet, 97 i detta exempel.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H
    
#### Range Proofs

I alla beräkningar ovan förlitar vi oss på att alla belopp är positiva. Introduktionen av negativa belopp skulle vara
extremt problematiskt då man skulle kunna skapa nya pengar i varje transaktion.

Till exempel skulle man kunna skapa en transaktion med inmatningen 2 och utmatningar 5 och -3 och fortfarande
ha en balanserad transaktion. Detta kan inte upptäcklas enkelt eftersom punkten `x*H` ser ut som vilken annan punkt
som helst på kurvan även om _x_ är negativt.

För att lösa detta problem använder MimbleWimble sig av ett kryptografiskt koncept som kallas "range proofs" (som också härstammar
från Confidential Transactions): ett bevis på att ett tal befinner sig inom ett visst intervall utan att avsölja talet.
Vi kommer inte att förklara range proofs; du behöver endast veta att för varje `r*G + v*H` kan vi skapa ett bevis som visar
att _v_ är större än noll och inte orsakar overflow.

Det är även viktigt att notera att både värdet 113 och värdet 28 måste vara kända för att kunna skapa ett giltigt range proof.
Anledningen till detta och en mer utförlig beskrivning av range proofs är förklarat i 
[range proof-pappret](https://eprint.iacr.org/2017/1066.pdf).

#### Sammanställningen av allt

En MimbleWimble-transaktion inkluderar följande:

* En mängd inmatningar som refererar till och spenderar en mängd föregående utmatningar.
* En mängd nya utmatningar som inkluderar:
  * Ett belopp och en bländande faktor (vilket bara är en ny privat nyckel) multiplicerade på en kurva och adderade
  till att bli `r*G + v*H`.
  * Ett range proof som visar att v är icke-negativt.
* En tydlig transaktionsavgift i klartext.
* En signatur vars privata nyckel beräknas genom att ta överskottsbeloppet (summan av alla utmatningar och 
avgiften minus inmatningarna).

### Block och kedjetillstånd

Vi förklarade ovan hur MimbleWimble-transaktioner kan erbjuda starka anonymitetsgarantier samtidigt som de
upprätthåller egenskaperna för en giltig blockkedja, d.v.s en transaktion skapar inte pengar och ägandebevis är
fastställt med privata nycklar.

MimbleWimble-blockformatet bygger på detta genom att introducera ett till koncept: _genomskärning_. Med detta
får en MimbleWimble-kedja: 

* Extremt bra skalbarhet då den stora majoriteten av transaktionsinformation kan elimineras på lång sikt utan att
kompromissa säkerhet.
* Ytterligare anonymitet genom att blanda och ta bort transaktionsinformation.
* Förmågan att effektivt synkronisera sig med resten av nätverket för nya noder.

#### Transaktionsaggregation

Kom igåg att en transaktion består av följande:

* En mängd inmatningar som refererar till och spenderar en mängd föregående utmatningar
* En mängd nya utmatningar (Pedersen commitments)
* En transaktionskärna som består av:
  * överskottsbelopp 
  * transaktionssignatur
  
En transaktion signeras och signaturen inkluderas i en transaktionskärna. Signaturen genereras genom att använda 
överskottsbeloppet som en publik nyckel för att bevisa att beloppen summeras till 0:

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H
    
Den publika nyckeln i detta exempel är `28*G`.

Vi kan säga att följande är sant för alla giltiga transaktioner (vi ignorerar avgifter för enkelhetens skull):

    summa(utmatningar) - summa(inmatningar) = överskottsbelopp
    
Detsamma gäller för blocken själva när vi inser att ett block helt enkelt är en mängd aggregerade inmatningar, utmatningar, och
transaktionskärnor. Vi kan summera transaktionsutmatningarna, subtrahera summan av transaktionsinmatningarna, och jämföra
det resulterande Pedersen commitment med summan av överskottsbeloppen:

    summa(utmatningar) - summa(inmatningar) = summa(överskottsbelopp)
    
    
Något förenklat, (återigen ignorerar vi transaktionsavgifter) kan vi säga att MimbleWimble-block kan betraktas precis som
MimbleWimble-transaktioner.

##### Kärn-offset

Det finns ett subtilt problem med MimbleWimble-block och transaktioner som beskrivet ovan. Det är möjligt (och i vissa fall
trivialt) att rekonstruera de konstituerande transaktionerna i ett block. Detta är naturligtvis dåligt för integriteten.
Detta är "delmängdsproblemet": given en mängd inmatningar, utmatningar, och transaktionskärnor kommer någon delmängd av detta
kunna kombineras för att rekonstruera en giltig transaktion.

Till exempel, vi har följande två transaktioner:

    (inmatning1, inmatning2) -> (utmatning1), (kärna1)
    (inmatning3) -> (utmatning2), (kärna2)
    
Vi kan aggregera dem till följande block:

    (inmatning1, inmatning2, inmatning3) -> (utmatning1, utmatning2), (kärna1, kärna2)
    
Det är trivialt att testa alla möjliga kombinationer och återskapa en av transaktionerna (där summan lyckas bli noll).

    (inmatning1, inmatning2) -> (utmatning1), (kärna1)
    
Vi vet också att allt som kvarstår kan användas för att rekonstruera den andra giltiga transaktionen:

    (inmatning3) -> (utmatning2), (kärna2)
    
För att mildra detta inkluderar vi ett _kärn-offset_ med varje överskottsbelopp. Detta är en bländande faktor som måste
tilläggas överskottsbeloppet för att verifiera att det summeras till noll:

    summa(utmatningar) - summa(inmatningar) = överskottsbelopp + kärn-offset
    
Vi "separerar" nyckeln `k` till `k1 + k2` under transaktionsbyggandet. För ett överskottsbelopp `(k1+k2)*G` publicerar vi
`k1*G` (överskottet) och `k2` (offset) och signerar transaktionen med `k1*G` som tidigare. Under block-konstruktionen
kan vi enkelt summera alla `k2`-offset för att generera ett aggregat-offset för alla transaktioner i blocket. `k2`-offsetet
för en individuell transaktion är omöjlig att få fram.

#### Genomskärning

Blocks låter miners sätta ihop flera transaktioner till en enstaka mängd som läggs till på kedjan. I följande
block-representationer som innerhåller tre transaktioner visar vi endast in- och utmatningarna. Inmatningar refererar till
föregående utmatningar som de spenderar. Föregående utmatningar markeras med _x_.

    I1(x1) --- O1
            |- O2
            
    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5
            
Vi lägger märke till följande två egenskaper:

* Inom detta block är vissa utmatningar spenderade direkt av inkluderade inmatningar (I3 spenderar O2, och I4 spenderar O3).
* Transaktionernas struktur spelar faktiskt ingen roll. Eftersom alla transaktioner individuellt summeras till noll
måste summan av alla transaktionsinmatningar och utmatningar summera till noll.

Liknande en transaktion, allt som behöver kontrolleras i ett block är att ägandebevis (vilket kommer från transaktionskärnorna)
och att blocket i helhet inte skapade pengar ur tomma intet. Således kan matchande inmatningar och utmatningar elimineras, då
deras sammansatta påverkan är noll. Detta leder till följande, mycket mer kompakta block:

    I1(x1) | O1
    I2(x2) | O4
           | O5
           
Notera att all transaktionsstruktur har eliminerats och att ordningen av in- och utmatningar inte längre spelar någon roll.
Summan av alla in- och utmatningar garanteras fortfarande vara noll.

Ett block består av:

* En block-header.
* En lista av alla inmatningar som kvarstår efter genomskärning.
* En lista av alla utmatningar som kvarstår efter genomskärning.
* Ett enstaka kärn-offset som skyddar hela blocket.
* Transaktionskärnor för varje transaktion som innehåller:
  * Publika nyckeln `r*G` erhållen genom summation av alla commitments.
  * Signaturerna genererade genom överskottsbeloppet.
  * Mining-avgiften
  
Med denna struktur erbjuder ett MimbleWimble-block extremt bra integritetsgarantier:

* Mellanliggande transaktioner är endast representerade av sina transaktionskärnor.
* Alla utmatningar ser likadana ut: väldigt stora tal som inte går att skilja åt på något meningsfullt sätt.
Om någon vill exkludera en specifik utmatning är de tvungna att exkludera alla.
* All transaktionsstruktur har borttagits vilket gör det omöjligt att se vilka in- och utmatningar som passar ihop.

Men ändå kan allting valideras!

#### Genomskärning hela vägen

Vi går tillbaka till blocket i föregående exempel. Utmatningarna x1 och x2 som spenderades av I1 och I2 måste ha
dykt upp tidigare i blockkedjan. Efter att detta block adderas till blockkedjan kan de utmatningarna tillsammans med
I1 och I2 alla tas bort från blockkedjan eftersom de nu är mellanliggande transaktioner.

Vi slutleder att kedjetillståndet kan (bortsett från block-headers) vid varje tidspunkt sammanfattas med endast dessa tre ting:

1. Den totala mängden mynt skapade genom mining.
2. Den kompletta mängden av oförbrukade utmatningar (UTXO).
3. Transaktionskärnorna för varje transaktion.

Det första kan härledas genom att endast observera block-höjden.

Både mängden av oförbrukade utmatningar och transaktionskärnorna är extremt kompakta. Detta har två följder:

* En nod i en MimbleWimble-blockkedja får en väldigt liten kedja att behöva ta vara på.
* När en ny nod ansluter sig till närverket krävs det väldigt lite information för att den ska bygga kedjan.

Dessutom kan man inte manipulera mängden av de oförbrukade utmatningarna. Tar man bort ett element ändras summan av
de bländande faktorerna och in- och utmatningarna matchar inte längre varandra.

### Slutsats

I detta dokument gick vi igenom de grundläggande principerna för en MimbleWimble-blockkedja. Genom att använda egenskaperna
för addition i kryptografi med elliptiska kurvor kan vi skapa fullständigt förmörkade transaktioner som ändå kan valideras.
Genom att generalisera dessa egenskaper till block kan vi eliminera en stor mängd blockkedjeinformation vilket medför
väldigt bra skalbarhet.
