# Introdução ao Mimblewimble e ao Grin

*Leia isto em outros idiomas: [English](../intro.md), [Español](intro_ES.md), [Nederlands](intro_NL.md), [Русский](intro_RU.md), [日本語](intro_JP.md), [Deutsch](intro_DE.md), [Portuguese](intro_PT-BR.md), [Korean](intro_KR.md), [简体中文](intro_ZH-CN.md).*

O Mimblewimble é um formato e protocolo blockchain que fornece ótima escalabilidade, privacidade e fungibilidade, para isso contando com primitivas criptográficas fortes. Ele aborda as lacunas existentes em quase todos as implementações blockchain atuais.

O Grin é um projeto de software de código aberto que implementa um blockchain Mimblewimble e preenche os vãos necessários para se construir um blockchain e uma criptomoeda completos.

O principal objetivo e as características do projeto Grin são:

* Privacidade por padrão. Isto permite fungibilidade completa sem impedir a capacidade de divulgação seletiva de informações quando necessário.
* Escalabilidade, sobretudo quanto ao número de usuários e minimamente com relação ao número de transações (<100 byte `núcleo`), resultando em uma grande economia de espaço quando comparado a outros blockchains.
* Criptografia forte e comprovada. O Mimblewimble se baseia apenas em Criptografia de Curva Elíptica testada e experimentada há décadas.
* Simplicidade no design o que facilita a auditoria e manutenção com o tempo.
* Direcionado pela comunidade, incentivando a descentralização da mineração.

## Amarra-Língua para Todos

Este documento destina-se a leitores com uma boa compreensão de blockchains e criptografia básica. Tendo isto em mente, tentamos explicar o desenvolvimento técnico do Mimblewimble e como ele é aplicado no Grin. Acreditamos que este documento seja compreensível para a maioria dos leitores tecnicamente conscientes. Nosso objetivo é incentivá-los a se interessar pelo Grin e contribuir da maneira que for possível.

Para alcançar este objetivo, apresentaremos os principais conceitos necessários para uma boa compreensão do Grin, sendo esta uma implementação do Mimblewimble. Vamos começar com uma breve descrição de algumas propriedades relevantes da Criptografia de Curva Elíptica (CCE) de forma a sedimentar a fundação em que o Grin é baseado e, em seguida, descrever todos os elementos-chave de transações e blocos do blockchain Mimblewimble.

### Um Pouquinho sobre Curvas Elípticas

Começamos com uma breve cartilha sobre Criptografia de Curva Elíptica, revisando apenas propriedades necessárias para entender como o Mimblewimble funciona e sem se aprofundar muito nos meandros da CCE. Para os leitores que gostariam de mergulhar mais fundo nesses pressupostos, existem outras oportunidades para [aprender mais](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

Uma Curva Elíptica para fins de criptografia é simplesmente um grande conjunto de pontos que nós chamaremos de _C_. Estes pontos podem ser adicionados, subtraídos ou multiplicados por inteiros (também chamados de escalares). Dado um inteiro _k_ e usando a operação de multiplicação escalar, podemos calcular `k*H`, que também é um ponto da curva _C_. Dado outro inteiro _j_ também podemos calcular `(k+j)*H`, que é igual a `k*H + j*H`. As operações de adição e multiplicação escalar em uma curva elíptica mantem as propriedades comutativa e associativa da adição e multiplicação:

    (k+j)*H = k*H + j*H

Em CCE, se escolhermos um número muito grande _k_ como uma chave privada, `k*H` é considerada a chave pública correspondente. Mesmo tendo conhecimento do valor da chave pública `k*H`, deduzir _k_ é quase impossível (ou em outras palavras, enquanto a multiplicação é trivial, a "divisão" por pontos da curva é extremamente difícil).

A fórmula anterior `(k+j)*H = k*H + j*H`, com _k_ e _j_ ambos sendo chaves privadas, demonstra que uma chave pública obtida a partir da adição de duas chaves privadas (`(k+j)*H`) é idêntica à adição das chaves públicas para cada uma dessas duas chaves privadas (`k*H + j*H`). No blockchain do Bitcoin, as carteiras Hierárquicas Determinísticas dependem fortemente desse princípio. O Mimblewimble e a implementação do Grin dependem também.

### Transacionando com o Mimblewimble

A estrutura das transações demonstra um princípio crucial do Mimblewimble: a garantia forte de privacidade e confidencialidade.

A validação das transações do Mimblewimble depende de duas propriedades básicas:

* **Verificação de somas zero.** A soma das saídas menos as entradas é sempre igual a zero, provando que a transação não criou novos fundos, _sem revelar os montantes reais_.
* **Posse de chaves privadas.** Como na maioria das outras criptomoedas, a propriedade sobre as saídas das transações é garantida pela posse de chaves privadas CCE. Contudo, a prova de que uma entidade possui essas chaves privadas não é obtida através da assinatura direta da transação.

As próximas seções sobre saldo, propriedade, troco e provas detalham como essas duas propriedades fundamentais são alcançadas.

#### Saldo

Com base nas propriedades da CCE descritas acima, pode-se obscurecer os montantes em uma transação.

Se _v_ é o montante de uma entrada ou saída de transação e _H_ é uma curva elíptica, podemos simplesmente incorporar `v*H` ao invés de _v_ em uma transação. Isso funciona porque usando as operações de CCE, permanecemos capazes de validar que a soma das saídas de uma transação é igual à soma das entradas:

    v1 + v2 = v3  =>  v1*H + v2*H = v3*H

A verificação dessa propriedade em toda transação permite que o protocolo verifique que uma transação não cria dinheiro do nada, sem de fato saber quais são seus montantes. No entanto, há um número finito de montantes utilizáveis e pode-se tentar cada um deles no intuito de adivinhar o montante da transação. Além disso, ter conhecimento de v1 (vindo de uma transação anterior, por exemplo) e do montante resultante `v1*H` revela todas as saídas com montante v1 do blockchain inteiro. Por estas razões, introduzimos uma segunda curva elíptica _G_ (na pratica _G_ é apenas outro ponto gerador no mesmo grupo de curvas que _H_) e uma chave privada _r_ usada como *fator de cegueira* (blinding factor).

Um montante de entrada ou saída em uma transação pode ser expresso como:

    r*G + v*H

Onde:

* _r_ é uma chave privada usada como fator de cegueira, _G_ é uma curva elíptica e seu produto `r*G` é a chave pública para _r_ em _G_.
* _v_ é o montante de uma entrada ou saída e _H_ é uma outra curva elíptica.

Nem _v_ ou _r_ podem ser deduzidos, tirando proveito das propriedades fundamentais da Criptografia de Curva Elíptica. `r*G + v*H` é chamado de _Compromisso de Pedersen_ (Pedersen Commitment).

Como exemplo, vamos supor que queremos construir uma transação com duas entradas e uma saída. Nós temos (ignorando taxas):

* ve1 e ve2 como montantes de entrada.
* vs3 como montante de saída.

De modo que:

    ve1 + ve2 = vs3

Gerando uma chave privada como fator de cegueira para cada montante de entrada e substituindo, na equação anterior, cada montante por seus respectivos Compromisso de Pedersen, obtemos:

    (re1*G + ve1*H) + (re2*G + ve2*H) = (rs3*G + vs3*H)

Que, como consequência, requer:

    re1 + re2 = rs3

Este é o primeiro pilar do Mimblewimble: a aritmética necessária para validar uma transação pode ser feita sem conhecer nenhum dos montantes.

Como nota final, esta ideia é, na verdade, derivada das [Transações Confidenciais](https://elementsproject.org/features/confidential-transactions/investigation) de  Greg Maxwell, que por si derivou de uma proposta de Adam Back para montantes homomórficos aplicados ao Bitcoin.

#### Propriedade

Na seção anterior, introduzimos uma chave privada como um fator de cegueira para obscurecer os montantes da transação. A segunda perspicácia do Mimblewimble é que esta chave privada pode ser aproveitada para provar a propriedade do montante.

Alice lhe envia 3 moedas e, para obscurecer essa quantia, você escolheu 28 como seu fator de cegueira (note que, na prática, o fator de cegueira sendo uma chave privada, é um número extremamente grande). Em algum lugar no blockchain, a seguinte saída aparece e só pode ser gasta por você:

    X = 28*G + 3*H

_X_, o resultado da adição, é visível para todos. O montante 3 só é conhecido por você e Alice, e 28 só é conhecido por você.

Para transferir novamente essas 3 moedas, o protocolo requer que 28 seja conhecido de alguma forma. Para demonstrar como isso funciona, digamos que você queira transferir essas 3 moedas para Carol. Você precisa construir uma transação simples de tal forma que:

    Xi => Y

Onde _Xi_ é uma entrada que gasta sua saída _X_ e Y é a saída de Carol. Não há como construir tal transação e contabilizá-la sem saber sua chave privada 28. De fato, para Carol contabilizar essa transação, ela precisa saber tanto o montante enviado quanto a sua chave privada, de modo que:

    Y - Xi = (28*G + 3*H) - (28*G + 3*H) = 0*G + 0*H

Ao verificar que tudo foi zerado, podemos garantir que nenhum dinheiro novo foi criado.

Espera aí! Pare! Agora você tomou conhecimento da chave privada presente na saída de Carol (que, nesse caso, deve ser a mesma que a sua para que o saldo bata) e assim você poderia roubar de volta o dinheiro da Carol!

Para resolver isso, a Carol usa uma chave privada escolhida por ela. Vamos dizer que ela escolha 113, e o que vai para o blockchain é:

    Y - Xi = (113*G + 3*H) - (28*G + 3*H) = 85*G + 0*H

Agora a soma da transação não é mais zero e temos um _montante excedente_ (excess value) em _G_ (85), que é o resultado da soma de todos os fatores de cegueira. Mas, dado que `85*G` é uma chave pública válida na curva elíptica _C_, com chave privada 85, para qualquer x e y, logo `x*G + y*H` será uma chave pública válida em _G_ somente se `y = 0`.

Então, tudo o que o protocolo precisa verificar é que (`Y - Xi`) é uma chave pública válida em _G_ e que as partes envolvidas na transação conhecem coletivamente a chave privada (85 em nossa transação com Carol). A maneira mais simples de fazer isso é exigir uma assinatura construída com o montante excedente (85), que por conseguinte valida que:

* As partes envolvidas na transação conhecem coletivamente a chave privada e
* A soma das saídas da transação, menos as entradas, é zero (porque somente uma chave pública válida, correspondente à chave privada, verificará a assinatura).

Esta assinatura, anexada a todas as transações, juntamente com alguns dados adicionais (como as taxas de mineração), é chamada de _núcleo da transação_ (transaction kernel) e é verificada por todos os validadores.

#### Alguns Pontos Refinados

Esta seção detalha a construção de transações discutindo como o troco deve ser introduzido e a exigência de provas de intervalo para que todos os montantes sejam comprovadamente não-negativos. Nenhum destes pontos é absolutamente necessário para entender o Mimblewimble e o Grin, por isso, se estiver com pressa, sinta-se à vontade para pular direto para [Juntando Tudo] (#juntando-tudo).

##### Troco

Digamos que você só queira enviar 2 moedas para Carol das 3 que você recebeu de Alice. Para fazer isso, você enviaria a 1 moeda restante de volta para si mesmo como troco. Você gera outra chave privada (digamos 12) como fator de cegueira para proteger sua saída de troco. Carol usa sua própria chave privada conforme antes.

    Saída de troco:    12*G + 1*H
    Saída da Carol:    113*G + 2*H

O que fica no blockchain é algo muito parecido com o anterior. E a assinatura é novamente construída com o montante excedente, 97 neste exemplo.

    (12*G + 1*H) + (113*G + 2*H) - (28*G + 3*H) = 97*G + 0*H

##### Prova de Intervalo

Em todos os cálculos acima, trabalhamos com montantes de transação sempre positivos. A introdução de quantidades negativas seria extremamente problemática, uma vez que poderia-se criar novos fundos em todas as transações.

Por exemplo, pode-se criar uma transação com uma entrada de montante 2 e saídas de montantes 5 e -3 obtendo uma transação devidamente estruturada, de acordo com a definição das seções anteriores. Isso não pode ser detectado facilmente, porque mesmo sendo _x_ negativo, o ponto correspondente `x.H` na curva se assemelha a qualquer outro.

Para resolver este problema, o Mimblewimble utiliza outro conceito criptográfico (também proveniente de transações confidenciais) chamado prova de intervalo: uma prova que um número se enquadra dentro de um determinado intervalo, sem revelar este número. Nós não iremos elaborar sobre a prova de intervalo, basta saber que para qualquer `r.G + v.H` podemos construir uma prova que mostrará que _v_ é maior que zero e não sofre estouro numérico.

Também é importante notar que, para criar uma prova de intervalo válida a partir do exemplo acima, ambos os montantes 113 e 28 usados na criação e assinatura do montante excedente devem ser conhecidos. A razão disto, assim como uma descrição aprofundada da prova de intervalo, estão mais detalhadas no [artigo sobre provas de intervalo] (https://eprint.iacr.org/2017/1066.pdf).

#### Juntando Tudo

Uma transação Mimblewimble inclui o seguinte:

* Um conjunto de entradas, que referencia e gasta um conjunto de saídas anteriores.
* Um conjunto de novas saídas que incluem:
  * Um montante e um fator de cegueira (que é apenas uma nova chave privada) multiplicados em suas curvas e somados `r.G + v.H`.
  * Uma prova de intervalo que mostra que v é não-negativo.
* Uma taxa de transação explícita, isto é, aparente.
* Uma assinatura, calculada a partir do montante de cegueira excedente (a soma de todas as saídas mais a taxa, menos as entradas) e usando-a como chave privada.

### Blocos e Estado da Cadeia

Nós explicamos acima como as transações do Mimblewimble podem fornecer forte garantia de anonimato, mantendo as propriedades necessárias de um blockchain válido, ou seja, uma transação não cria dinheiro e a prova de propriedade é estabelecida através de chaves privadas.

O formato de bloco Mimblewimble se baseia nisso, introduzindo um conceito: _corte-completo_ (cut-through). Com esta adição, uma cadeia Mimblewimble ganha:

* Ótima escalabilidade, já que a grande maioria dos dados de transações podem ser eliminados com o tempo, sem comprometer a segurança.
* Mais anonimato, misturando e removendo dados de transações.
* E a capacidade de novos nodos sincronizarem com o resto da rede eficientemente.

#### Agregando Transações

Lembre-se de que uma transação consiste no seguinte -

* um conjunto de entradas que referencia e gasta um conjunto de saídas anteriores
* um conjunto de novas saídas (compromissos de Pedersen)
* um núcleo de transação, consistindo de
  * excedente do núcleo (compromisso de Pedersen zerado)
  * assinatura da transação (usando o excedente do núcleo como chave pública)

Uma transação é assinada e a assinatura incluída em um _núcleo de transação_ (transaction kernel). A assinatura é gerada usando o _excedente do núcleo_ (kernel excess) como chave pública provando que a soma da transação é igual a 0.

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

A chave pública neste exemplo é `28*G`.

Podemos dizer que o seguinte é verdadeiro para qualquer transação válida (ignorando taxas por simplicidade) -

    soma(saídas) - soma(entradas) = excedente do núcleo

O mesmo vale para os próprios blocos, uma vez que percebemos que um bloco é simplesmente um conjunto de entradas, saídas e núcleos de transação agregados. Podemos somar as saídas da transação, subtrair a soma das entradas da transação e comparar o compromisso Pedersen resultante com a soma dos excedentes do núcleo -

    soma(saídas) - soma(entradas) = soma(excedente do núcleo)

Simplificando um pouco (ignorando novamente as taxas de transação), podemos dizer que os blocos Mimblewimble podem ser tratados exatamente como transações Mimblewimble.

##### Deslocamentos do Núcleo

Há um problema sutil nos blocos e transações do Mimblewimble, conforme descrito acima. É possível (e em alguns casos, é trivial) reconstruir as transações constituintes de um bloco. Isso é claramente ruim para a privacidade. Este é o problema do "subconjunto" - dado um conjunto de entradas, saídas e núcleos de transação, um subconjunto destes recombinará para reconstruir uma transação válida.

Por exemplo, dadas as duas transações a seguir -

    (ent1, ent2) -> (sai1), (nucl1)
    (ent3) -> (sai2), (nucl2)

Podemos agregá-los no seguinte bloco (ou transação agregada) -

    (ent1, ent2, ent3) -> (sai1, sai2), (nucl1, nucl2)

É trivialmente fácil tentar todas as permutações possíveis para recuperar uma das transações (onde a mesma tem soma zero) -

    (ent1, ent2) -> (sai1), (nucl1)

Sabemos também que tudo que resta pode ser usado para reconstruir com validade a outra transação -

    (ent3) -> (sai2), (nucl2)

Para mitigar isso, incluímos um _deslocamento do núcleo_ (kernel offset) em cada núcleo de transação. Este é um fator de cegueira (chave privada) que precisa ser adicionado novamente ao núcleo excedente para que os compromissos se anulem -

    soma(saídas) - soma(entradas) = excedente do núcleo + deslocamento do núcleo

Quando agregamos transações em um bloco, armazenamos um _único_ deslocamento de agregação no cabeçalho do bloco. E agora temos um único deslocamento que não pode ser decomposto nos deslocamentos do núcleo da transação individual e as transações não podem mais ser reconstruídas -

    soma(saídas) - soma(entradas) = soma(excedente do núcleo) + deslocamento do núcleo

Nós "dividimos" a chave `k` em` k1+k2` durante a construção da transação. Para um núcleo de transação `(k1+k2)*G` publicamos` k1*G` (o excedente) e `k2` (o deslocamento) e assinamos a transação com `k1*G` como antes. Durante a construção de blocos, podemos simplesmente somar os deslocamentos `k2` para gerar um único deslocamento agregado de `k2` para cobrir todas as transações no bloco. O deslocamento `k2` para qualquer transação individual é irrecuperável.

#### Corte-completo

Os blocos permitem que os mineradores montem várias transações em um único conjunto que é adicionado à cadeia. Nas seguintes representações de bloco, contendo 3 transações, nós só mostramos entradas e saídas de transações. Entradas referenciam as saídas que elas gastam. Uma saída incluída em um bloco anterior é marcado com um x minúsculo.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

Notamos as duas propriedades a seguir:

* Dentro deste bloco, algumas saídas são gastas diretamente pelas entradas incluídas (I3 gasta O2 e I4 gasta O3).
* A estrutura de cada transação não importa realmente. Como todas as transações se anulam individualmente, a soma de todas as entradas e saídas de transação deve ser zero.

Similarmente a uma transação, tudo o que precisa ser verificado em um bloco é que a propriedade foi provada (que vem de _núcleos de transação_) e que o bloco inteiro não acrescentou nenhum suprimento de dinheiro (além do que é permitido pela coinbase). Portanto, as entradas e saídas correspondentes podem ser eliminadas, já que sua contribuição para a soma global se anula. O que leva ao seguinte bloco, muito mais compacto:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Note que toda a estrutura da transação foi eliminada e a ordem de entradas e saídas não importa mais. No entanto, a soma de todas as saídas neste bloco, menos as entradas, ainda é garantida como sendo zero.

Um bloco é simplesmente constituído de:

* Um cabeçalho de bloco.
* A lista de entradas restantes após o corte-completo.
* A lista de saídas restantes após o corte-completo.
* Um único deslocamento do núcleo para cobrir o bloco inteiro.
* Os núcleos de transação contendo, para cada transação:
  * A chave pública `r*G` obtida do somatório de todos os compromissos.
  * As assinaturas geradas usando o montante excedente.
  * A taxa de mineração.

Quando estruturado dessa maneira, um bloco Mimblewimble oferece garantias extremamente boas de privacidade:

* Transações intermediárias (corte-completo) serão representadas apenas por seus núcleos de transação.
* Todas as saídas se assemelham: apenas números muito grandes que são impossíveis de distinguir um do outro. Se alguém quisesse excluir algumas saídas, este teria que excluir todas.
* Toda estrutura da transação foi removida, tornando impossível dizer qual saída foi combinado com cada entrada.

E, no entanto, tudo ainda permanece válido!

#### Corte-completo Inteiramente

Voltando ao bloco do exemplo anterior, as saídas x1 e x2, gastas por I1 e I2, devem ter aparecido anteriormente no blockchain. Então, após a adição deste bloco, essas saídas, assim como I1 e I2, também podem ser removidas da cadeia global, pois não contribuem para a soma global.

Generalizando, concluímos que o estado da cadeia (excluindo cabeçalhos) a qualquer momento pode ser resumido simplesmente pelas seguintes informações:

1. A quantidade total de moedas criadas pela mineração na cadeia.
1. O conjunto completo de saídas não gastas.
1. Os núcleos de transações para cada transação.

A primeira informação pode ser deduzida usando apenas a altura do bloco (sua distância do bloco de gênese). E tanto as saídas não gastas quanto os núcleos de transação são extremamente compactos. Isso tem 2 consequências importantes:

* O estado que um determinado nó do blockchain Mimblewimble precisa manter é muito pequeno (na ordem de alguns gigabytes para um blockchain do tamanho do bitcoin, e potencialmente otimizável para algumas centenas de megabytes).
* Quando um novo nó se une à rede que constrói uma cadeia Mimblewimble, a quantidade de informação que precisa ser transferida também é muito pequena.

Além disso, o conjunto completo de saídas não gastas não pode ser adulterado, mesmo somente adicionando ou removendo uma saída. Isso faria com que a soma de todos os fatores de cegueira nos núcleos de transação diferissem da soma dos fatores de cegueira nas saídas.

### Conclusão

Neste documento, cobrimos os princípios básicos subjacentes a um blockchain Mimblewimble. Usando as propriedades de adição da Criptografia de Curva Elíptica, construímos transações completamente opacas, mas que ainda assim podem ser corretamente validadas. E ao generalizar essas propriedades em blocos, podemos eliminar uma grande quantidade de dados do blockchain, permitindo uma grande escalabilidade bem como a rápida sincronização de novos pares.
