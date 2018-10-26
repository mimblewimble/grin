# Введение в Мимблвимбл и Grin

*На других языках: [English](intro.md), [简体中文](intro.zh-cn.md), [Español](intro_ES.md).*

МимблВимбл это формат и протокол блокчейна, предоставляющий
исключительную масштабируемость, приватность и обезличенность криптовалюты,
опираясь на сильные криптографические примитивы.
Этот протокол призван исправить недостаток, присущий почти всем текущим
реализациям блокчейна.

Grin это проект с открытым исходым кодом, реалищующий блокчейн МимблВимбл
и закрывает пробелы, требуемые для окончательного внедрения блокчейна и криптовалют.

Основными целями и характеристиками Grin являются:

* Приватность по-умолчанию. Возможность использовать криптовалюту
анонимно, так и выборочно раскрывать часть данных.
* Масштабируется в основном пропорционально количеству пользователей
и минимально с количчеством транзакций, что приводит к значительной
экономии дискового пространства по сравнению с другими блокчейнами.
* Сильная и доказанная криптография. МимблВимбл опирается на проверенные
временем Эллиптические Кривые, используемые и тестируемые десятилетиями
* Простой дизайн системы упрощает аудит и дальнейшую поддержку
* Ведётся сообществом, использует ASIC-устойчивый алгоритм майнинга
(Cuckoo Cycle), приветствуя децентрализацию майнеров.

### Завязывание Языка для Чайников

Этот документ ориентирован на читателей с хорошим пониманием
блокчейнов и основ криптографии. Имея это в виду, мы попытаемся
объяснить техническую основу МимблВимбла и как он применён в Grin.
Мы надеемся этот документ будет понятен большинству технически-
ориентированныэх читателей, ведь наша цель это заинтересовать вас
использовать Grin и принимать участие в проекте всеми возможными способами.

Чтобы достигнуть этой цели, мы опишем основные идеи, требуемые для хорошего
понимания принципов работы Grin как реализации МимблВимбла. Мы начнем с
кратого описания некоторых из основных свойст Криптографии на Эллиптических
Кривых, чтобы заложить фундамент, на котором основан Grin, и затем расскажем
о ключевых частях транзакций и блоков МимблВимбла.

### Немного Эллиптических Кривых

Мы начнём с кратого примера Криптографии на Эллиптических Кривых,
рассмотрев свойства, необходимые для понимания работы МимблВимбла и без
излишнего погружения в тонкости данного вида криптографии.

Эллиптическая Кривая, для целей криптографии, это просто большое множество точек,
которые мы назовём _C_. Эти точки можно складывать, вычитать или умножать на целые числа
(так же называемые скалярами).
Пусть _k_ является целым числом, тогда, используя скалярное умножение, мы можем вычислить
`k*H`, что так же является точкой на кривой _C_. Пусть дано другое целое число _j_,
тогда мы также можем вычислить `(k+j)*H`, что равняется `k*H + j*H`.
Сложение и скалярное умноэение на Эллиптической Кривой удовлетворяет свойствам коммутативности и
ассоциативности сложения и умножения:

    (k+j)*H = k*H + j*H

В Эллиптической Криптографии, если мы выберем большое значение _k_ как публичный ключ,
тогда произведение `k*H` станет соответствующим публичным ключём.
Даже если кто-то знает значение публичного ключа `k*H`, вычисление _k_ близко к невозможному
(другими словами, не смотря на тривиальность умножения, деление точек Эллиптической Кривой является
крайне сложным). 

Предыдущая формула `(k+j)*H = k*H + j*H`, где _k_ и _j_ хранятся в тайне, 
показывает, что публичный ключ может быть получен путём сложения двух приватных ключей
и является идентичным сложению двух соответствующих публичных ключей. Например, в Биткоин
работа Детерминистических Иерархичных (HD wallets) кошельков всецело
основана на этом принципе. МимблВимбл и Grin тоже используют это свойство.

### Создание Транзакций в МимблВимбл

Структура транзакций указывает на ключевые принципы МимблВимбла: нерушимую
гарантию приватности и конфиденциальности.

Проверка транзакций МимблВимбла опирается на два основных свойства:

* **Проверка нулевых сумм.** Сумма выходов минус сумма входов всегда равняется нулю,
  это доказывает, что транзакция не создаёт новых монет, при этом _без раскрытия реальных сумм переводов_.
* **Владение приватным ключём.** Как и у большинства других криптовалют, владение
  выходами транзакции гарантируется владением приватного ключа. Однако доказательство того,
  что некто владет приватным ключём, достигается иначе, нежели простой подписью транзакции.

Далее будет рассказано, как вычисляется баланс кошелька, 
проверяется владение, образуется "сдача" и будет показано, как перечисленные выше свойства
достигаются.

#### Баланс

Основываясь на свойствах Эллиптических Кривых (ЭК), некто может сокрыть 
количество отправляемых монет в транзакции.

Пусть _v_ это значение входа или выхода транзакции и _H_ это Эллиптическая Кривая, тогда
мы можем просто подставить значение `v*H` вместо _v_ в транзакцию. Это работает благодаря тому,
что используя операции на Эллиптической Кривой, мы сможем удостовериться, что сумма
выходов транзакции равняется сумме её входов:

    v1 + v2 = v3  =>  v1*H + v2*H = v3*H

Проверка этого свойства для каждой транзакции позволяет протоколу удостовериться,
что транзакция не создаёт новые монеты из воздуха, при этом не раскрывая
количества передаваемых в транзакциях монет.

Однако, количество пригодных для использования количеств монет конечно и злоумышленник может 
попытаться угадать передаваемое количетсво монет путём перебора. Кроме того, знание _v1_ 
(например из предыдущей транзакции) и конечного значения `v1*H`, раскрывает значения
всех выходов всех транзакций, которые используют _v1_. Из-за этого мы введём вторую
Эллиптическую Кривую _G_ (на самом деле _G_ это просто ещё один генератор группы, образованной той же самой кривой _H_)
и некий приватный ключ _r_ используемый как *фактор сокрытия*.

Таким образом, значения входов и выходов транзакции могут быть выражены как:

    r*G + v*H

Где:

* _r_ приватный ключ, используемый как фактор сокрытия, _G_ это Эллиптическая Кривая и
  произведение `r*G` это публичный ключ для _r_ на кривой _G_.
* _v_ это значение входа или выхода транзакции, а _H_ это другая ЭК.

Опираясь на ключевые свойства ЭК, ни _v_ ни _r_ не могут быть вычислены. Произведение
`r*G + v*H` называется _Обязательство Педерсена_.

В качестве примера, предположим, что мы хотим создать транзакцию с двумя входами и одним выходом,
тогда (без учёта комиссий):

* vi1 и vi2 это входы.
* vo3 это выход.

Такие, что:

    vi1 + vi2 = vo3

Создав приватные ключи как факторы сокрытия для каждого из значений и заменив их на
соответствующие Обязательства Педерсена в предыдущем уравнении, мы получим другое уравнение:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vo3*H)

Которое, как следствие, требует, чтобы:

    ri1 + ri2 = ro3

Это первый из столпов МимблВимбла: вычисления, требуемые для валидации транзакции,
могут быть совершены без раскрытия количеств монет, передаваемых этими транзакциями.

Примечательно, что эта идея была выведена из 
[Конфиденциальных Транзакций](https://www.elementsproject.org/elements/confidential-transactions/) Грега Максвелла,
которые, в свою очередь, сами основаны на предложении Адама Бэка для гомоморфных значений, применимых к
Биткоину.

#### Владение

Выше мы ввели приватный ключ в качестве фактора сокрытия, чтобы засекретить информацию о 
количестве передаваемых транзакцией монет. Вторая идея, которую предоставляет МимблВимбл, это 
то, что этот же самый ключ может использоваться для доказательства владения монетами.

Алиса отправляет вам 3 монеты и, чтобы засекретить количество, вы выбрали 28 как ваш
фактор сокрытия (заметим, что на практике фактор сокрытия, будучи приватным ключём, 
является очень большим числом). Тогда где-то в блокчейне должен быть следующий выход (UTXO),
доступный для траты только вами:

    X = 28*G + 3*H

Сумма _X_ является видимой для всех, а значение 3 известно только вам и Алисе. Ну а число 28 известно только вам.

Чтобы передать эти 3 монеты снова, протокол требует, чтобы число 28 было каким-то образом раскрыто.
Чтобы показать принцип работы, допустим, что вы хотите передать те же 3 моенты Кэрол.
Тогда вам нужно создать простую транзакцию, такую, что:

    Xi => Y

Где _Xi_ это выход, который тратит ваш вход _X_, а Y это выход для Кэрол. Не существует способа создать
такую транзакыию без знания вашего приватного ключа 28. Разумеется, если Кэрол решит принять монеты из этой
транзации, ей нужно будет узнать как значение вашего приватного ключа, так и значение, которое этой
транзакцией переводится. Таким образом:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H

Проверяя, что всё свелось к нулю, мы в очередной раз убедимся, что новых монет создано не было.

Но постойте-ка! Теперь вы знаете значение приватного ключа из выхода Кэрол (которое, в этом случае, должно быть
такое же, как и у вас, чтобы свести сумму в ноль) и тогда вы можете украсть деньги у Кэрол назад!

Для решения этой проблемы, Кэрол использует приватный ключ, который выбрала сама.
Например, она выбрала 133, тогда в блокчейн будет записано:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H

Эта сумма (транзакция) больше не сводится к нулю и мы имеем _избыточное_ значение на _G_ (85), 
которое является результатом сложения всех факторов сокрытия. Но из-за того, что
произведение `85*G` будет являться корректным публичным ключем на ЭК _G_ с приватным ключем 85,
для любого x и y, только если `y = 0`, сумма `x*G + y*H` будет являться публичным ключём на _G_.

Всё что нужно, это проверить, что (`Y - Xi`) - валидный публичный ключ на кривой _G_ и
участники транзакции совместно обладают приватным ключём (85 в случае транзакции с Кэрол).
Простейший способ достичь этого, это если потребовать создавать некую подпись избыточного значения (85),
которая будет удостоверять, что:

* Участники транзакции совместно знают приватный ключ, и
* Сумма выходов транзакции минус сумма входов, равняется нулю
  (потому что только валидный публичный ключ будет удовлетворять этой подписи)
  
Эта подпись, прикрепляемая к каждой транзакции, совместно с некоей дополнительной информацией 
(например комиссиями майнеров) называется _ядром транзакции_ и должна проверяться всеми валидаторами.

#### Некоторые Уточнения

Этот раздел уточняет процесс создания транзакций, обсудив то, как образуется 
"сдача" и требования для доказательств неотрицательности значений. Ничего из этого не 
требуется для понимания МимблВимбла и Grin, так что если вы спешите, можете спокойной переходить
к разделу [Всё Вместе](#всё-вместе).

##### Сдача

Допустим вы хотите отправить 2 монеты Кэрол из трёх монет, которые вы получили от 
Алисы. Чтобы это сделать, вы отправите остаток из 1 монеты назад к себе в качестве сдачи.
Для этого создайте другой приватный ключ (например 12) в качестве фактора сокрытия, чтобы защитить ваш
выход сдачи. Кэрол использует свой приватный ключ как и ранее.

    Выход для сдачи:     12*G + 1*H
    Выход для Кэрол:    113*G + 2*H

Тогда в блокчейн попадёт кое-что уже нам знакомое, а подпись опять-таки построена 
на избыточном значении, 97 в этом примере.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H

##### Доказательства Интервала

Во всех вычислениях, проделанных выше, мы опираемся на тот факт, что количества передаваемых
монет в транзакциях всегда являются положительным числом. Если допустить использовать отрицательные
значения передаваемых монет, это вызовет много проблем и позволит создавать новые монеты в каждой транзакции.

Например, некто может создать транзакцию со входом 2 и выходами 5 и -3 монет и
всё равно получить хорошо сбалансированную транзакцию, согласно формулам выше.
Трудно будет обрнаружить такие случаи, поскольку даже если _x_ отрицательно, 
соответствующая точка `x*H` на кривой является неотличимой от других.

To solve this problem, MimbleWimble leverages another cryptographic concept (also
coming from Confidential Transactions) called
range proofs: a proof that a number falls within a given range, without revealing
the number. We won't elaborate on the range proof, but you just need to know
that for any `r.G + v.H` we can build a proof that will show that _v_ is greater than
zero and does not overflow.

It's also important to note that in order to create a valid range proof from the example above, both of the values 113 and 28 used in creating and signing for the excess value must be known. The reason for this, as well as a more detailed description of range proofs are further detailed in the [range proof paper](https://eprint.iacr.org/2017/1066.pdf).

#### Putting It All Together

A MimbleWimble transaction includes the following:

* A set of inputs, that reference and spend a set of previous outputs.
* A set of new outputs that include:
  * A value and a blinding factor (which is just a new private key) multiplied on
  a curve and summed to be `r.G + v.H`.
  * A range proof that shows that v is non-negative.
* An explicit transaction fee, in clear.
* A signature, computed by taking the excess blinding value (the sum of all
  outputs plus the fee, minus the inputs) and using it as a private key.

### Blocks and Chain State

We've explained above how MimbleWimble transactions can provide
strong anonymity guarantees while maintaining the properties required for a valid
blockchain, i.e., a transaction does not create money and proof of ownership
is established through private keys.

The MimbleWimble block format builds on this by introducing one additional
concept: _cut-through_. With this addition, a MimbleWimble chain gains:

* Extremely good scalability, as the great majority of transaction data can be
  eliminated over time, without compromising security.
* Further anonymity by mixing and removing transaction data.
* And the ability for new nodes to sync up with the rest of the network very
  efficiently.

#### Transaction Aggregation

Recall that a transaction consists of the following -

* a set of inputs that reference and spent a set of previous outputs
* a set of new outputs (Pedersen commitments)
* a transaction kernel, consisting of
  * kernel excess (Pedersen commitment to zero)
  * transaction signature (using kernel excess as public key)

A tx is signed and the signature included in a _transaction kernel_. The signature is generated using the _kernel excess_ as a public key proving that the transaction sums to 0.

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

The public key in this example being `28*G`.

We can say the following is true for any valid transaction (ignoring fees for simplicity) -

    sum(outputs) - sum(inputs) = kernel_excess

The same holds true for blocks themselves once we realize a block is simply a set of aggregated inputs, outputs and transaction kernels. We can sum the tx outputs, subtract the sum of the tx inputs and compare the resulting Pedersen commitment to the sum of the kernel excesses -

    sum(outputs) - sum(inputs) = sum(kernel_excess)

Simplifying slightly, (again ignoring transaction fees) we can say that MimbleWimble blocks can be treated exactly as MimbleWimble transactions.

##### Kernel Offsets

There is a subtle problem with MimbleWimble blocks and transactions as described above. It is possible (and in some cases trivial) to reconstruct the constituent transactions in a block. This is clearly bad for privacy. This is the "subset" problem - given a set of inputs, outputs and transaction kernels a subset of these will recombine to  reconstruct a valid transaction.

For example, given the following two transactions -

    (in1, in2) -> (out1), (kern1)
    (in3) -> (out2), (kern2)

We can aggregate them into the following block (or aggregate transaction) -

    (in1, in2, in3) -> (out1, out2), (kern1, kern2)

It is trivially easy to try all possible permutations to recover one of the transactions (where it sums successfully to zero) -

    (in1, in2) -> (out1), (kern1)

We also know that everything remaining can be used to reconstruct the other valid transaction -

    (in3) -> (out2), (kern2)

To mitigate this we include a _kernel offset_ with every transaction kernel. This is a blinding factor (private key) that needs to be added back to the kernel excess to verify the commitments sum to zero -

    sum(outputs) - sum(inputs) = kernel_excess + kernel_offset

When we aggregate transactions in a block we store a _single_ aggregate offset in the block header. And now we have a single offset that cannot be decomposed into the individual transaction kernel offsets and the transactions can no longer be reconstructed -

    sum(outputs) - sum(inputs) = sum(kernel_excess) + kernel_offset

We "split" the key `k` into `k1+k2` during transaction construction. For a transaction kernel `(k1+k2)*G` we publish `k1*G` (the excess) and `k2` (the offset) and sign the transaction with `k1*G` as before.
During block construction we can simply sum the `k2` offsets to generate a single aggregate `k2` offset to cover all transactions in the block. The `k2` offset for any individual transaction is unrecoverable.

#### Cut-through

Blocks let miners assemble multiple transactions into a single set that's added
to the chain. In the following block representations, containing 3 transactions,
we only show inputs and
outputs of transactions. Inputs reference outputs they spend. An output included
in a previous block is marked with a lower-case x.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

We notice the two following properties:

* Within this block, some outputs are directly spent by included inputs (I3
  spends O2 and I4 spends O3).
* The structure of each transaction does not actually matter. As all transactions
  individually sum to zero, the sum of all transaction inputs and outputs must be zero.

Similarly to a transaction, all that needs to be checked in a block is that ownership
has been proven (which comes from _transaction kernels_) and that the whole block did
not add any money supply (other than what's allowed by the coinbase).
Therefore, matching inputs and outputs can be eliminated, as their contribution to the overall
sum cancels out. Which leads to the following, much more compact block:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Note that all transaction structure has been eliminated and the order of inputs and
outputs does not matter anymore. However, the sum of all outputs in this block,
minus the inputs, is still guaranteed to be zero.

A block is simply built from:

* A block header.
* The list of inputs remaining after cut-through.
* The list of outputs remaining after cut-through.
* A single kernel offset to cover the full block.
* The transaction kernels containing, for each transaction:
  * The public key `r*G` obtained from the summation of all the commitments.
  * The signatures generated using the excess value.
  * The mining fee.

When structured this way, a MimbleWimble block offers extremely good privacy
guarantees:

* Intermediate (cut-through) transactions will be represented only by their transaction kernels.
* All outputs look the same: just very large numbers that are impossible to
  differentiate from one another. If one wanted to exclude some outputs, they'd have
  to exclude all.
* All transaction structure has been removed, making it impossible to tell which output
  was matched with each input.

And yet, it all still validates!

#### Cut-through All The Way

Going back to the previous example block, outputs x1 and x2, spent by I1 and
I2, must have appeared previously in the blockchain. So after the addition of
this block, those outputs as well as I1 and I2 can also be removed from the
overall chain, as they do not contribute to the overall sum.

Generalizing, we conclude that the chain state (excluding headers) at any point
in time can be summarized by just these pieces of information:

1. The total amount of coins created by mining in the chain.
2. The complete set of unspent outputs.
3. The transactions kernels for each transaction.

The first piece of information can be deduced just using the block
height (its distance from the genesis block). And both the unspent outputs and the
transaction kernels are extremely compact. This has 2 important consequences:

* The state a given node in a MimbleWimble blockchain needs to maintain is very
  small (on the order of a few gigabytes for a bitcoin-sized blockchain, and
  potentially optimizable to a few hundreds of megabytes).
* When a new node joins a network building up a MimbleWimble chain, the amount of
  information that needs to be transferred is also very small.

In addition, the complete set of unspent outputs cannot be tampered with, even
only by adding or removing an output. Doing so would cause the summation of all
blinding factors in the transaction kernels to differ from the summation of blinding
factors in the outputs.

### Conclusion

In this document we covered the basic principles that underlie a MimbleWimble
blockchain. By using the addition properties of Elliptic Curve Cryptography, we're
able to build transactions that are completely opaque but can still be properly
validated. And by generalizing those properties to blocks, we can eliminate a large
amount of blockchain data, allowing for great scaling and fast sync of new peers.
