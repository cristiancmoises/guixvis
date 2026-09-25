# guixvis

Explorador interativo de pacotes e visualizador de dependências para o
**GNU Guix** — uma interface de terminal (Rust + ratatui) no espírito do
pacvis do Arch: pesquise **qualquer** pacote, veja tudo que está
**relacionado** a ele, com busca fuzzy rápida e uma interface polida,
100% pelo teclado.

[English](README.md) · [Guia de uso](docs/usage.md) ·
[Mudanças na versão 0.8.0](docs/releases/0.8.0.md) · [Segurança](SECURITY.md)

A versão 0.8.0 mantém o pacote selecionado ao trocar de aba, pesquisa todas as
dependências e dependências reversas alcançáveis e abre o grafo do terminal
com um único nível. Versões diferentes e variantes privadas do Guix continuam
distintas no terminal, no navegador e no Emacs.

## Uma nota do autor

Fiz isso para o meu próprio uso. Rodo Guix nas minhas máquinas, vivia esquecendo
como os pacotes se ligam, e queria algo mais rápido do que passar a tarde
grepeando `guix show` — por isso o guixvis é todo de teclado, escuro por padrão,
e o grafo foi feito para ser lido no terminal.

É a ferramenta que eu uso todo dia. Sinta-se à vontade para usar também, e para
mudar o que não te agradar.

## Recursos

- Busca global por nome e sinopse, com ranking fuzzy e vários termos.
- Detalhes do pacote, licenças, página e localização no código do Guix.
- Árvores de dependências e dependências reversas, com filtros locais por
  nome e versão sobre todos os objetos alcançáveis.
- Inputs comuns (`I`), propagados (`P`) e nativos (`N`), sem juntar
  variantes só porque têm o mesmo nome.
- Grafo com profundidade inicial 1, arestas focadas, lista navegável e detalhes
  completos do pacote selecionado.
- Cache binário com origem identificada: executável do Guix, sistema e commits
  de todos os canais.
- Comandos do Guix para revisar e copiar. Navegar não instala nem remove nada.

## Capturas de tela

Capturas da versão 0.8.0 instalada localmente, usando o índice real do Guix.

![Guixvis 0.8: busca no terminal com versões distintas do Emacs](assets/guixvis-tui-overview-0.8.0.png)

![Guixvis 0.8: grafo de um nível ao lado da lista de pacotes](assets/guixvis-tui-graph-0.8.0.png)

## Interface web

O `guixvis web` serve o mesmo explorador como um site local em
<http://127.0.0.1:8787>: busca fuzzy no topo, painel de detalhes com chips
clicáveis de pacotes relacionados e o grafo interativo em que cada bolha é
um pacote — clique em uma para abrir a visão dela. Links diretos
incluem ID do pacote e token do snapshot para não misturar variantes de mesmo
nome. Links antigos só com nome continuam funcionando; referências exatas de
um snapshot substituído pedem uma nova busca. São links para o serviço local.
Clique com o botão direito no grafo ou use o botão
**Back** visível para voltar um passo no guixvis, restaurando pacote,
profundidade e direção. Na primeira visão interna, o botão fica desativado
e não sai do site. O layout se adapta a celulares.

Clique em **Packages** para manter os resultados na tela, com versão,
sinopse, licença e contagens de dependências. A lista mostra até 100
resultados ordenados; refine a busca quando atingir esse limite. O botão
**Graph** volta ao grafo. Nos detalhes, os comandos Guix ficam disponíveis
para revisão e cópia.

Em **Graph style**, escolha **Bubbles** ou **Rectangles**. O navegador lembra
a escolha. As bolhas mostram rótulos da raiz, da seleção e dos nós maiores
que couberem; os retângulos trazem o nome dentro da forma, abreviado quando
necessário. Passe o mouse sobre um nó ou selecione-o pelo teclado para
inspecioná-lo. Arraste com o botão principal para mover um nó, arraste o fundo
para deslocar o grafo, use a roda para ampliar ou faça pinça na tela sensível
ao toque. Esses gestos não abrem outro pacote.

![Guixvis 0.8: grafo web com bolhas](assets/guixvis-web-bubbles-0.8.0.png)

![Guixvis 0.8: retângulos com zoom do navegador em 100%](assets/guixvis-web-rectangles-0.8.0.png)

![Guixvis 0.8: dependências do Python em tela estreita](assets/guixvis-web-mobile-0.8.0.png)

## Temas

As duas interfaces trazem nove temas de cores: **dark** (padrão da TUI), **one**,
**light**, **dracula**, **nord**, **gruvbox-dark**, **tokyo-night**,
**catppuccin-mocha** e **tron** — este último é preto puro com bolhas neon, que
é o que você quer num painel OLED de madrugada.

- TUI: no modo Navigate, pressione `T` para alternar (o tema ativo aparece na barra de
  status). A escolha fica em `$XDG_CONFIG_HOME/guixvis/theme`, normalmente
  `~/.config/guixvis/theme`. Use `guixvis --theme nord` para mudar só nesta
  execução. `NO_COLOR` é respeitado com um tema em tons de cinza.
- Interface web: escolha o tema no seletor do topo; a escolha fica salva
  entre as sessões. **System**, o padrão para novos usuários, acompanha o
  modo claro/escuro do sistema. A preferência por movimento reduzido também
  é respeitada, e o grafo para de redesenhar quando estabiliza.

## Requisitos

- GNU Guix (`guix` no `PATH`, ou defina `GUIX` apontando para o seu perfil).
- Rust 1.88+ (edition 2021) para compilar a partir do código-fonte.

## Instalação

```sh
git clone https://codeberg.org/berkeley/guixvis guixvis
cd guixvis
cargo install --locked --features web --path .  # instala em ~/.cargo/bin
# ou, para deixar direto no seu PATH:
cargo install --locked --features web --root ~/.local --path .
guixvis
```

Sem `--features web`, a instalação inclui apenas a interface de terminal.
O navegador e o cliente nativo do Emacs precisam desse recurso.

Espelhos: `github.com/cristiancmoises/guixvis`,
`git.securityops.co/cristiancmoises/guixvis`,
`git.securityops.com.br/cristiancmoises/guixvis`.

### Pelo canal Guix da securityops

O guixvis está empacotado no
[canal securityops](https://git.securityops.com.br/cristiancmoises/securityops-channel)
(`(securityops packages apps)`). Adicione o canal ao `channels.scm` (veja o
README do canal), rode `guix pull` e então:

```sh
guix install guixvis
```

O canal compila o guixvis a partir do código-fonte com o registro Cargo
vendado (offline, `cargo --frozen`), incluindo a interface web
(`guixvis web`).

### Artefatos de release (.zupt)

Os fontes publicados em cada forja saem como `guixvis-<versão>.zupt`, um arquivo
[zupt](https://git.securityops.com.br/cristiancmoises/zupt) gerado no nível
máximo de compressão e **sem senha**, então qualquer pessoa consegue abrir.
Depois da publicação, baixe o arquivo e `SHA256SUMS` na
[release 0.8.0](https://codeberg.org/berkeley/guixvis/releases/tag/v0.8.0):

```sh
sha256sum -c SHA256SUMS
zupt test    guixvis-0.8.0.zupt    # verifica a integridade do arquivo
zupt list    guixvis-0.8.0.zupt    # confira os caminhos antes de extrair
zupt extract guixvis-0.8.0.zupt    # cria ./guixvis-0.8.0/
```

Depois é compilar normalmente:

```sh
cd guixvis-0.8.0
cargo build --locked --release --features web
```

O `zupt` vem do canal securityops (`guix install zupt`) ou dos repositórios
dele. As releases até a 0.3.0 foram repacotadas de `.tar.gz` para `.zupt`, então
todas as versões agora saem no mesmo formato; o canal Guix mantém um `.tar.gz`
simples como fonte do pacote, porque o daemon de build precisa descompactar sem
ferramentas extras. Esses insumos internos são separados dos downloads de
release: os novos arquivos publicados usam somente `.zupt`. Os links de fontes
gerados automaticamente pelas forjas ainda podem oferecer outros formatos.
O [guia de publicação](docs/releasing.md) registra como empacotar e conferir.

### Emacs

Carregue `elisp/guixvis.el` e inicie `guixvis web` em um terminal. Com
`M-x guixvis-search`, você pesquisa numa tabela nativa do Emacs sem bloquear
o editor. `M-x guixvis-package` abre um pacote pelo nome. Use `RET` para os
detalhes, `g` para atualizar, `s` para buscar, `w` para copiar um comando e
`b` para abrir o navegador. Os comandos de pacote nunca rodam automaticamente.

```elisp
(add-to-list 'load-path "/caminho/para/guixvis/elisp")
(require 'guixvis)
;; Opcional, se você usa Emacs-Guix:
;; (guixvis-popup-install)
```

`M-x guixvis` continua abrindo a TUI num buffer `term`, agora reutilizando
o processo quando ele já está rodando. Configure `guixvis-web-url` se usar
outra porta local. O [guia de uso](docs/usage.md#emacs) detalha as opções.

O arquivo fica aqui, e não no emacs-guix, para que as entradas do menu só
apareçam para quem realmente tem o programa instalado (veja
[guix/emacs-guix#40](https://codeberg.org/guix/emacs-guix/pulls/40)).

## Uso

```
guixvis              inicia o explorador (constrói o índice na 1ª execução)
guixvis --rebuild    força a reconstrução do índice
guixvis --theme nord  escolhe um tema de terminal para esta execução
guixvis web          inicia o site local e a API para o Emacs
guixvis --help       todas as opções
```

### Teclas

O cabeçalho informa se você está no modo **Search** (pesquisa) ou **Navigate**
(navegação). A Visão geral começa em Search. Pressione `/` em qualquer aba
para pesquisar; nesse modo, todos os caracteres imprimíveis entram na busca,
inclusive letras de atalhos e números. `Enter` ou `Esc` encerra a edição
sem abrir um pacote nem apagar o texto.

| Tecla | Ação |
|---|---|
| `Tab` / `Shift+Tab` | trocar de aba nos dois modos |
| setas, `PgUp` / `PgDn` | mover o cursor da lista atual |
| `Ctrl+U` | limpar a busca da aba atual |
| `F1` / `Ctrl+C` | ajuda / sair nos dois modos |
| `/` | entrar em Search |
| `1`–`4`, `d` / `r` / `v` | escolher aba em Navigate |
| `Enter` | expandir árvore ou seguir nó em Navigate |
| `Esc` | em Navigate: limpar o filtro; depois, voltar pelo histórico do grafo |
| `+` / `−` | profundidade do grafo, de 1 a 8, em Navigate |
| `e` / `l` | modo das arestas / rótulos do grafo |
| `[` / `]` | rolar os detalhes do pacote no grafo |
| `g` | voltar ao pacote da Visão geral no grafo |
| `T` / `R` / `o` | tema / reconstruir índice / página do pacote em Navigate |
| `?` / `q` | ajuda / sair em Navigate |

### Abas

1. **Overview** — busca fuzzy global por nome e sinopse.
2. **Dependencies** — pesquisa todas as dependências alcançáveis do pacote,
   inclusive variantes privadas e as três categorias de inputs.
3. **Reverse deps** — pesquisa todas as dependências reversas alcançáveis
   neste índice.
4. **Graph** — filtra somente os nós projetados. Para uma busca completa das
   relações, use as abas de dependências.

Os filtros locais procuram palavras literais em nome e versão, sem distinguir
maiúsculas; todas devem corresponder. Não usam o ranking fuzzy da Visão geral.
Cada aba preserva sua busca e cursor. Mover-se numa árvore não muda o pacote
da Visão geral; escolher outro resultado nela reinicia as vistas relacionadas.

A árvore distingue caminhos repetidos e ciclos. O filtro busca o conjunto
completo mesmo com ramos fechados ou quando a vista expandida atinge seu
limite de linhas. Esse limite não esconde dependências diretas.

## Como funciona

Ao iniciar, o Guixvis carrega o snapshot ou executa o indexador Guile embutido.
Ele começa em `fold-packages` e segue os objetos reais presentes em `inputs`,
`propagated-inputs` e `native-inputs`. Objetos com o mesmo nome não são
fundidos, e variantes privadas alcançadas pelos inputs também aparecem.

São os inputs declarados dos pacotes para o sistema Guix selecionado, não um
grafo de derivações, closure da store, plano de compilação cruzada ou lista de
pacotes instalados. Falhas na extração produzem diagnósticos e um aviso de
índice incompleto.

O cache fica em `~/.cache/guixvis/index-v5.bin`, ou em `$XDG_CACHE_HOME`.
A escrita é atômica. A versão 0.8 reconstrói o índice uma vez e preserva o cache
v4 antigo. Snapshots corrompidos são colocados em quarentena, não apagados.

A escolha do Guix segue `GUIX` (perfil ou executável), `PATH` e os perfis
padrão, nessa ordem. Os links simbólicos do lançador são preservados, pois
resolvê-los pode remover extensões de canais. A origem registra esse caminho,
o sistema e os nomes/commits de todos os canais. Se não for possível verificá-la,
ou se `GUIX_PACKAGE_PATH` carregar módulos locais mutáveis, a interface avisa.
IDs de pacotes só têm significado junto com o token do snapshot.

## Lendo o grafo

O terminal começa na profundidade **1**, com arestas focadas no pacote
selecionado. Não é preciso pressionar `−` várias vezes para limpar a vista
inicial. O traçado usa pontos Unicode finos, sem preencher blocos inteiros.
Em terminais largos, o desenho fica ao lado de uma lista navegável;
nos estreitos, aparece a lista. Nome e versão completos quebram em linhas no
painel de detalhes; `[` e `]` permitem rolá-lo.

`Enter` segue um pacote. `Esc` sai de Search, depois limpa o filtro e então
volta pelo histórico do grafo. `g` retorna à referência da Visão geral.
Profundidade, seleção e filtro sobrevivem à troca de abas.

`e` alterna arestas focadas, todas e nenhuma; `l` alterna os rótulos do
desenho. A lista continua disponível quando os nomes não cabem no desenho.
As marcas `I/P/N` preservam todas as categorias encontradas na descoberta.

O grafo tem limites: 200 nós **incluindo a raiz**, 3.000 arestas e um teto de
trabalho na travessia. A interface distingue totais exibidos, omitidos e
desconhecidos; desconhecido não vira zero. Para buscar relações completas,
use Dependencies ou Reverse deps. Na web, a profundidade inicial continua 2.

## Desempenho

Use `cargo run --release --example bench` para medir cache, busca e layout
na sua máquina. `node examples/bench-web.cjs` mede o layout do navegador.

Nesta máquina, com build release e 41.746 objetos de oito canais, a extração
levou 8,73 s e a leitura do snapshot de 18,6 MB levou 109 ms. Oito buscas tiveram
média de 2,63 ms (melhor de 20 execuções por consulta, limite de 500 resultados).
Os layouts amostrados de 200 nós levaram cerca de 13–15 ms, com 300 iterações.

São medições locais, não garantias de latência nem uma comparação direta de
ganho com índices antigos. A versão 0.8 preserva mais objetos e identidades
exatas. A travessia de relações e o layout do terminal rodam fora da renderização;
filtrar não recalcula o layout a cada quadro.

## Segurança

O guixvis roda na sua máquina e lê a sua instalação do Guix, então a interface
web é deliberadamente chata quanto a alcance:

- escuta apenas em `127.0.0.1` e recusa conexões cujo par não é loopback;
- recusa requisições cujo `Host` não seja `localhost`/`127.0.0.1`/`::1`
  (proteção contra DNS rebinding) e cujo `Origin` ou `Sec-Fetch-Site` indique
  outro site;
- serve CSP estrita (`default-src 'self'`, em todas as rotas, não só no
  documento), `X-Content-Type-Options`,
  `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`,
  `Cross-Origin-Resource-Policy` e `Cache-Control: no-store` na API;
- valida os nomes de pacote vindos da URL, limita a busca a 200 caracteres, a
  profundidade a 1–8 e o grafo a 200 nós incluindo a raiz, com
  concorrência máxima de quatro;
- grava o script Guile embutido num diretório privado `0700` como arquivo `0600`
  (o diretório temporário do sistema é gravável por todos) e recusa arquivos de
  cache absurdamente grandes antes de lê-los;
- limita o corpo das requisições a 8 KB: uma API somente leitura não tem o que
  fazer com um corpo.

A API não tem login e foi feita para uso local. Os comandos aparecem como
texto para você revisar e copiar; o servidor não instala nem remove pacotes.
Não exponha o serviço por um proxy público. Veja [SECURITY.md](SECURITY.md)
para os limites dessa proteção.

## Desenvolvimento

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features            # testes unitários, fixtures e web
node --test tests/web_graph_tests.cjs tests/web_app_tests.cjs
emacs -Q --batch -L elisp -l elisp/guixvis.el -l elisp/guixvis-tests.el -f ert-run-tests-batch-and-exit
cargo test --test live_guix_tests -- --ignored   # testes reais contra o Guix
```

Estrutura: `src/index.rs` (índice em memória + BFS), `src/search.rs` (busca
fuzzy com nucleo), `src/indexer.rs` (subprocesso `guix repl`),
`src/cache.rs` (snapshot binário), `src/graph.rs` (extração do grafo + layout),
`src/app.rs` (estado + teclas), `src/ui/*` (renderização),
`data/guix-index.scm` (indexador Guile).

## Solução de problemas

- **"guix not found"** — exporte `GUIX=/caminho/do/seu-perfil` ou instale o
  Guix; o guixvis também procura nos caminhos padrão de perfil do sistema.
- **Primeira execução lenta** — a primeira construção compila os módulos
  Guile de pacotes; leva segundos com cache quente e alguns minutos frio.
  O progresso aparece ao vivo.
- **Dados desatualizados após `guix pull`** — reinicie o guixvis; o cache é
  reconstruído automaticamente quando a origem verificada muda.
- **Sem cores** — `NO_COLOR` é respeitado (tema em tons de cinza).
- **Indexador travado** — saia de Search e pressione `R`, ou reinicie com
  `guixvis --rebuild`. Confira o erro antes de remover qualquer cache.

## Licença

GPL-3.0-or-later. Veja [LICENSE](LICENSE).
