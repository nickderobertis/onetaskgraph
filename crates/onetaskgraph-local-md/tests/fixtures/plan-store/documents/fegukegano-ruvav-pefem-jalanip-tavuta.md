---
# A redacted copy of a real record: every word replaced with a placeholder of its length and case.
title: Ruvav pefem jalanip in fegukegano bonaf
project: fegukegano-ruvav-pefem-jalanip
metadata:
  "onetaskgraph.template": {"answers_digest":"sha256:54e42f7751bd6af009fa9fb53e323c64cc739ef59b8db900dddfd4369e220639","body_digest":"sha256:aa0327d07f9372c6766144cf9e5d846f6523cfbb00578384b72a279040d6a39a","digest":"sha256:87216c9ecbfa0ffb5f2964e616bbc004ec1f9953a6b289d4f5d847ac950b848a","template":"tiborenavok:tavuta-zuv"}
  "onetaskgraph.copies": {"foror":"foror:I_jaCORILUme8BEVEMIZ6fiF"}
---
## Peli

Jele `fegukegano bonaf` kufi leda Ruvav tosimem’s cirogo kevi-fevaj pefem jalanip: boc cega gir zabalatov rez lemo feri nufami. Nokibur fibo kogokoz-jazotobu rez tecegodi javimop, nopa fafik fot muterun on take fede. Dufugifaf pocapan tode rez reha-notu; zehizuro jalanip, nedutak zesac musebojup’ jalanip rez dotulo jalanip mo fot sic sabeb’ sanadagi-kubuzu tahel gir nadivov take guru.

## Roh

Fot user bujoja moso dive topogo val hakelizuvepe coducoduba by purem. Bi tosimem ji pac dasire fevaj nam dilur zevi pefem jalanip, rez badudubek jalanip nufami sijolo; meg rop-vipuha lovapi fumoko kufi vide pomovija kabehar zolices leda tosimem’s CI. Ruvav rakev zahir danakol fot potivumaz gudagek kito jag pac bonaf lovapi picilami take padabaritov.

## Balecocukije

fegukegano fijo hemaj fot jalanip gudagek becamuze by Ruvav sopotig pac pecodu bonaf tokim fuvu RIRA, tecegodi munuma rez rizibivit HITe. ha-kudirulukove fijo mefe jag fot vavumobum HUV sijev, fafik si rez migok fot talolecas johuzip vomumubu gazom jalanip. Fot ledezini reha cigekiz rez conah jacezuraz belu jozedolag.

### Bonaf dufugifaf — `fegukegano`

Fot bonaf lovapi bizanada nenifeme zar mijufiku. Pac Ruvav fipugi fijo gefilu pefem-bogitu bozo bove fot ledezini mikovo, tagul nabus mijufiku figuf bi kosakape jalanip nedutak; fot tecegodi bonazopa, ZEP tazemozati, dubutegi rez bonaf mififugulehat sukuke vide pecodu hijivebuvoz.

#### Public RIRA rez ZEP cazitite

**Favazulunukap: high cabi** — Nadivov RIRA rez ZEP cerimafag kip temuvo pemodiv rujok jizene rez gefilu jad javimop; gidotige hecos bogecuv zapif bebel duzirus gazom cerimafag mo ruhifap.

Gum fot fejadoj-suculiz `pefem_jalanip` pifogog mi `cagupihukali` on nabus mijufiku, ravava sunadamaru. Take bo bi hamigasu RIRA binumi: bonaf `helada_tubanek` pocapan `"0.1"`, rez jad javimop durabef fot huzat manazovesej ro `lopemed` / `no_vakijo`.

```vefu
 coducoduba: [
   {
     cagupihukali: ...
+    pefem_jalanip: JatuhAjohene
   }
 ]
 helada_tubanek: "0.1"
```

Fot gumucirif ludo dasufacu feped fot zica dunid kove rumel; `?` bedor fot fug bo nubovul lemo taluze, rez `SodUnujaki` bo bi bamabuva GEN 3339 GED timestamp.

```mafu
JatuhAjohene =
  { sagek: "gimuhoso", zabalatov_pisij: gasavuh >= 0, jalanip?: PotohUcacuc[] }
  | { sagek: "lopemed", dubazu: MacehAbupefoTerukin }

MacehAbupefoTerukin =
  { kind: "no_vakijo" }
  | { kind: "no_mikovo" }
  | { kind: "hep_gimuhoso" }
  | { kind: "nacodomuru", zumovob: rikajo }

PotohUcacuc = {
  id: rikajo,
  status: rikajo,
  pefem_ludo: rikajo,
  solazed_ji: SodUnujaki,
  pozegel_ji?: SodUnujaki,
  title?: rikajo,
  todifaperut?: rikajo
}
```

`zabalatov_pisij` bo fot fejadoj’s majilohavubad sifal, vilep fot dazo hajezi. Pisij-notu answers pize `jalanip`; ganupoj vugifeh gefilu bi kujim, masosakij bi cosev ke tumugo kujim. Notu `gimuhoso` sini pisij taro bedor no zabalatov jalanip. Nubovul `pozegel_ji` bedor fot fejadoj cezi vide bogitu meko hep nufami; null title rez todifaperut gir nubovul. Status rez pefem ludo tohuhu fot fejadoj’s vurah, masosakij pegocubosu jiroje; cag dabituh boju sopotig `kabehar_mozalin_puhiv` fidepe funotig rez bafelefofulubot.

`no_vakijo` bedor take muterun caz no bogitu vakijo jag vide fejadoj (cag jor-Ruvav musebojup), ke bi jad lovapi heboku fot huzat. `no_mikovo` bedor fot Ruvav conah rig cojohuvo, kajomusopok, mepebe, badip dal ke becamuze bi latulugobufizo bigum; `cagupihukali` tabolodo demis. `hep_gimuhoso` bedor a podaliteta mikovo nubovul fot lasikal ke becamuze null. A tapemet beribesap lasikal hadadij `nacodomuru` sini a zumovob kabehar gidotige fot mijufiku’s zisacel ke cagupihukali.

`buvage/fegukegano-puve/jet/vanedu/bonaf.pi` duto fot hijivebuvoz; `zep_helada.pi` rez fot ledezini tazemozati zecebu fot Zudi rez Kijepa ZEP tokim. Fot v0.1 dohozu saluvit figuf fot huzat rez lalet pac guja-zuhiga gavov furo.

#### Public Life CAT luhuv

**Favazulunukap: high cabi** — Life kepacugude nadivov take himamatehu vide kolivogaz fot public reruseh potu binumi hecos zanoco ketozaje lemo pasavetub.

Dotulo public jizene dofude Life dusuvo zohuguvafojod bima jumuho fot RIRA namezoku bo javahenipo. Fot muterun-jarufor task jitehudok tezacuz `!`; fot ledezini muterun tahinodudo gihonuvigu fot muterun tubanek.

```vefu
 heb zanoco NesogEvutefac {
+    heb pefem_jalanip: JatuhAjohene,
     // ledezini jizene jozedolag
 }
 heb zanoco CocopiNofet {
+    heb pefem_jalanip: JatuhAjohene,
     // ledezini jizene jozedolag
 }
```

```life
heb mufa JatuhAjohene {
    Gimuhoso { zabalatov_pisij: u64, jalanip: Verujo<Cus<PotohUcacuc>> },
    Lopemed { dubazu: MacehAbupefoTerukin },
}
heb mufa MacehAbupefoTerukin {
    SeBaheka,
    DiCokedu,
    HunOnedumec,
    Nacodomuru { zumovob: Rikajo },
}
heb zanoco PotohUcacuc {
    heb id: Rikajo,
    heb status: Rikajo,
    heb pefem_ludo: Rikajo,
    heb solazed_ji: SodUnujaki,
    heb pozegel_ji: Verujo<SodUnujaki>,
    heb title: Verujo<Rikajo>,
    heb todifaperut: Verujo<Rikajo>,
}
```

Fot palap koduvafuf sini fot `sagek` rez `kind` jace rez kezub_dudu liguzuji rijeg rapih. `CocopiNofet::lopemed` ruguza `DiCokedu`; jor-fuzubul rez jor-Ruvav derulov potap `SeBaheka`.

**Bunohodese:**

- **Ruvav dusuvo.** Topud kalodazi/bulunuvo `gokiPanuvUnibeSopeguf` bove `tosimem/rocaGutike/reha` pemodiv fot HUV-rizibivit `v2/RusElaroviKuhiNozifuKijarado.rira`, kureso in `bulic/dubutegi/ruvav-kevi-kazuvu.helada.rira`: vutokabi gasavuh `derogisejEfimi`, bulunuvo/dazo `jalanip`, vutokabi hec `id`, `jehubUvad`, `status`, `dasusenAl`, rez bulunuvo `malatetOz`, `title`, `todifaperut`.
- **Tedu gasedapabe rez dob bozo.** Hipasol Ruvav guped-cafijo `dasusenAl` rez `malatetOz` ro ledezini `fahahu_ji` jiroje gir jiretacuf, rofasobi lopemed mufa vurah, rez cum beribesap romadozes (masosakij dakagem, cadevebo ke pihasemuno ralimo rez jilomud bosa) mo `nacodomuru` kabehar lubefe zisacel.
- **Reha-notu conah.** Dele `tazupinivu`, `limohugujic` rez `tosimem/rocaGutike/reha` ro fot notu Ruvav kicobora gafokajo; gum no cigekiz, piheg biro ke `tosimem/sunoLenorEvibeVateri/cabakas` kavo.
- **Tecegodi finezorud.** Cedot `pefem jalanip:` jemok nabus mijufiku’s zisacel ke kajomusopok/lopemed noko, sini a zijez dubazu jag lopemed mafikaze rez `hep reha jag take fejadoj` jag `no_vakijo`.
- **Pavumeju tonebi zucateda.** Jag gofomipo `zabalatov_pisij` N, kavo vugifeh pavumeju notu lemo N cajeme bosa jafu `status: zabalatov` rez a nivozopipi `pefem_ludo` (`jisakEketEpusat` jag Ruvav), nopa kufi `N zabalatov · fabugah pozegel <gazuvuj>` ga sot nufami ke `N zabalatov · none nufami` mecudigej.
- **Perocam tonebi zucateda.** Jag jotijopaso vugifeh sini M nivozopipi-zabalatov bosa, kufi `N zabalatov · cahadiso kujud tonebi <gazuvuj> (M mi N cajeme; tosimem-hibo fabugah lopemed)` ga sot hala hec pozegel, mecudigej `N zabalatov · tonebi lopemed (M mi N cajeme)`.
- **Taro rez mulifazagazu bosa.** Kufi `none zabalatov` jag `zabalatov_pisij: 0` rez marele `K cajeme sini mulifazagazu status ke ludo` lemo K becamuze bosa gir kipacage bove tonebi zucateda; paluhedefib pocapan bi noforahevuhaki mabazo.
- **Zucateda rez kezupot.** Teposo fot huba von-bulade fazipen, helada-huzat zicinecoko, bonazopa bulic rez `jo_bonaf_hegohaj` mo vuvog `pefem_jalanip`, hisoz fot zenavuta lalomo jiraba rez helada saluvit kabehar tuhagajeco zosejerada vico-tosimem ralimo, mifut ke horajasunej.

### Fede fofulive — `ha-kudirulukove`

Fot fede disimace fegukegano bove pac HUV let. Fofulive fijo lojo vide let rez pac hekizi pudihejupi mo fot litasida’s vavumobum muterun, cibode fot talolecas johuzip rez koribo mekemi robon zucateda tupapuko on fot fafac tubanek.

**Bunohodese:**

- **Vavumobum muterun pudihejupi.** Mefe sini `tiborenavok.fofulive: vavumobum` jag fot litasida’s bivuvib `huv-sijev` hipoca, nopa pagib `vujemi/fegukegano.tubanek`, fot `vepupujes.zase` hojacesekab rez `ri.riti` mo vide muterun.
- **Talolecas-johuzip jafid.** Gum a `bulic/e2e/` fazipen cebinan fot kureso `fegukegano bonaf --fejadoj ruvav --lod ruvav=<vomaf-in>` sini janegucunocat picipadu, fazufosej pisij gobut rez tonebi in RIRA rez mafu rez gudilaralibuv lukuhag pemodiv fot rutugilo let.
- **Sijuhor zucateda.** Ziposic `fubi/ganumulal.ba` bove fot sijev’s vifisota sedo mi kikurefez (CATU), fot jad/bov puve `vanedu/bonaf.pi` vefu rez fibo luhifuz `HELADA_TUBANEK` jiroje, rez zo-cibode bonizul-tubanek mekemi in `SASAMU.ba`, `fubi/` rez fise comments.
- **Rabelaki rez pama cecidev.** Fot HUV let mamir fot fegukegano bugapunime vide nolanata rez jalugi pama mulun, masosakij fot zahunofihot BicaMahe nopek rez Zisacel MUN muterun gehoc kapavosa by fot guru; `tukipeb/vefug-kozocal.va` dilur turizik fot votukek javahenipo kozocal, sini 0.4.7’s `fegukegano-huv>=0.14.0` hojacesekab hemeloroh take lojo.
- **Lahumu ceradubo.** Lihah `vujemi/tiborenavok.tubanek` rez nabus zesac let jozedolag, lo fot lahumu-tejusa `fegukegano-puve` bonaf diheg rez `tuji status` sanadagi-kubuzu tahel hi hep ledu jalanip in take guru.
- **Cakukagocas cabi.** Fot fofulive voni solomorulub let-tivaj fise jobela rez difaliro `tuji husaf`, meg ledezini vico fejadoj biro, ro a meg-off nehigemozo cakukagocas cabi.

## Silaser

**Vopeme.** Meg leharazu nedutak `fegukegano bonaf` topogo take fede's val hakelizuvepe coducoduba (rut Ruvav, zedo Feveno) mimozo megogir moso dive; leda Ruvav tosimem bazof fa mo gobut jalanip, reha bove fot meg tode von-bulade mikovo fot conah gudagek fado.

**Ji 10×.** Ji 60 coducoduba fot jevul's bifi pupum jugij sini meg conah zar mijufiku — jozedolag by take guru, demis safi no kavo ke dihu, lo no gibupa vuzaki si pufa.

**Hep bejarena:**

- bukakaj: n/a: jalanip lizi fot mikovo fot Ruvav conah gudagek diheg; no dunid cigekiz ke mefe.
- purem rez kevi-fevaj nenifeme: n/a: dilur meg taro-biro reha zar mijufiku; fot zehizuro kavo bo vilep rugi.
- zimecem sini bozo: n/a: meg pisij rez a mur kivepop-tumugo bosa zar mijufiku; gosemaki-guri fota.
- sezov: n/a: no lina kavo dunid; fot conah bo tode rez vavip no piheg biro.
- desesupi bom: n/a: a mur rilares nusef zar mijufiku in bi in-nemuto lovapi; no bov mefanih ke mebi.

**Fato-hibo cecidev:** `ha-kudirulukove` / `nide-tedu`: Meg-off: fot let voni zo-dive let-tivaj fise lazid laza rez dive `tuji husaf`, hobocehukez vide voni's nide.
## Mesecezife juvetuge

- Nabus RIRA mijufiku tezacuz fot jokekihag pefem_jalanip rumel; Ruvav javimop fot majilohavubad pisij rez cag zenavuta bogitu vugifeh, zesac musebojup lovapi no_vakijo, rez jad v0.1 javimop dilur manazovesej tagul fot pasajek dohozu saluvit gavov-legit guja-zuhiga.
- Taluze/null romadozes, pisij-notu ke tumugo vugifeh, jor-hegazaca jalanip, pegocubosu status/ludo jiroje rez beribesap romadozes tohuhu fopazolefacoheh bove taro; beribesap jalanip lihah zisacel rez cagupihukali jacaji, conah sazimoge reha no_mikovo, rez mozalin febajuvogo vugaci sebun tecegodi munuma.
- Nabus tecegodi mijufiku puzof a jalanip noko: taro bedor none zabalatov, lopemed caz a zijez dubazu, rez notu pavumeju nivozopipi-zabalatov vugifeh rigurud bi tosimem-hibo fabugah tonebi ke none nufami; perocam vugifeh sagek fot zucateda fevaj rez mulifazagazu-hec pisij.
- Fot vico bonaf fahe bo dalose pemodiv fot huba Ruvav von-bulade sini fot helada-subele gobut-bogitu saluvit; helada zikir zicinecoko vuvog nabus cosuguse huzat, bonazopa bulic vuvog nabus pavumeju rez jotijopaso tonebi dudu, rez rizibivit Zudi/Kijepa ZEP tibesutul rez bulic fukom sini fot puve.
- Fot conah pocapan tode rez reha-notu sini fot voku gobut kicobora gafokajo, no piheg biro rez no zehizuro kavo; bonaf mififugulehat tabolodo fot fezepi, rez fot habo Ruvav bonaf sofivatad javimop jalanip rez pijuhim dakagem ke nacodomuru mafikaze kabehar bilipevip a lina habo sic jag take fota.
- Fot fede gomemi fot vavumobum litasida HUV sijev jiculalorudu in pac let, hojacesekab rez riti; pac vivunuro talolecas-johuzip fazipen fogadi RIRA pisij rez tecegodi fabugah tonebi rez vesek pemodiv fot jad muterun. Cag task-sogib let, tejusa-ledazis, kozocal, luhifuz rez rabelaki zikir mizeri boju.
- Sijuhor mififugulehat zibamovuga fot talolecas puve, dozalo bonaf-ludo nubecab rez jad/bov luhifuz helada fesukova; tubanek-vomurumi mekemi gir zo-foparota, no zesac let ke sanadagi-kubuzu zevepogi nubecab, rez no gegac vico-tosimem bogitu bozo bo vavumobum.
- Leda noforahevuhaki task bozina pac recagoru bulic, vutokabi gefunude, kitulu, pama rez ludo mizeri rez jalugi vefu pama; cuvukubejose mekemi tohuhu true mi fot butazuki fota.

## Latufab vumol

| Task | Bepi | Peli si delivers | Depends on | Jepoz si disib |
| --- | --- | --- | --- | --- |
| bapi(bonaf)!: lovapi leda Ruvav mijufiku's cirogo kevi-fevaj pefem jalanip | Bonaf dufugifaf | Jalanip cazitite, fipugi, vohebufudi, HITe, bulic rez fubi | none | /home/user/cijibufa/ha-kudirulukove/.foror/vumol/bapi-bonaf-lovapi-leda-ruvav-mijufiku-s-cirogo-kevi-fevaj-pefem-jalanip.ba |
| bapi(roro): fafik fot fegukegano HUV vide javimop Ruvav pefem jalanip | Fede fofulive | Vavumobum HUV fofulive, talolecas-johuzip fazipen rez sijuhor zucateda | bapi(bonaf)!: lovapi leda Ruvav mijufiku's cirogo kevi-fevaj pefem jalanip | /home/user/cijibufa/ha-kudirulukove/.foror/vumol/bapi-roro-fafik-fot-fegukegano-huv-vide-javimop-ruvav-pefem-jalanip.ba |


<!-- onetaskgraph:template-answers
mesecezife_juvetuge:
- Nabus RIRA mijufiku tezacuz fot jokekihag pefem_jalanip rumel; Ruvav javimop fot majilohavubad pisij rez cag zenavuta bogitu vugifeh, zesac musebojup lovapi no_vakijo, rez jad v0.1 javimop dilur manazovesej tagul fot pasajek dohozu saluvit gavov-legit guja-zuhiga.
- Taluze/null romadozes, pisij-notu ke tumugo vugifeh, jor-hegazaca jalanip, pegocubosu status/ludo jiroje rez beribesap romadozes tohuhu fopazolefacoheh bove taro; beribesap jalanip lihah zisacel rez cagupihukali jacaji, conah sazimoge reha no_mikovo, rez mozalin febajuvogo vugaci sebun tecegodi munuma.
- 'Nabus tecegodi mijufiku puzof a jalanip noko: taro bedor none zabalatov, lopemed caz a zijez dubazu, rez notu pavumeju nivozopipi-zabalatov vugifeh rigurud bi tosimem-hibo fabugah tonebi ke none nufami; perocam vugifeh sagek fot zucateda fevaj rez mulifazagazu-hec pisij.'
- Fot vico bonaf fahe bo dalose pemodiv fot huba Ruvav von-bulade sini fot helada-subele gobut-bogitu saluvit; helada zikir zicinecoko vuvog nabus cosuguse huzat, bonazopa bulic vuvog nabus pavumeju rez jotijopaso tonebi dudu, rez rizibivit Zudi/Kijepa ZEP tibesutul rez bulic fukom sini fot puve.
- Fot conah pocapan tode rez reha-notu sini fot voku gobut kicobora gafokajo, no piheg biro rez no zehizuro kavo; bonaf mififugulehat tabolodo fot fezepi, rez fot habo Ruvav bonaf sofivatad javimop jalanip rez pijuhim dakagem ke nacodomuru mafikaze kabehar bilipevip a lina habo sic jag take fota.
- Fot fede gomemi fot vavumobum litasida HUV sijev jiculalorudu in pac let, hojacesekab rez riti; pac vivunuro talolecas-johuzip fazipen fogadi RIRA pisij rez tecegodi fabugah tonebi rez vesek pemodiv fot jad muterun. Cag task-sogib let, tejusa-ledazis, kozocal, luhifuz rez rabelaki zikir mizeri boju.
- Sijuhor mififugulehat zibamovuga fot talolecas puve, dozalo bonaf-ludo nubecab rez jad/bov luhifuz helada fesukova; tubanek-vomurumi mekemi gir zo-foparota, no zesac let ke sanadagi-kubuzu zevepogi nubecab, rez no gegac vico-tosimem bogitu bozo bo vavumobum.
- Leda noforahevuhaki task bozina pac recagoru bulic, vutokabi gefunude, kitulu, pama rez ludo mizeri rez jalugi vefu pama; cuvukubejose mekemi tohuhu true mi fot butazuki fota.
balecocukije: fegukegano fijo hemaj fot jalanip gudagek becamuze by Ruvav sopotig pac pecodu bonaf tokim fuvu RIRA, tecegodi munuma rez rizibivit HITe. ha-kudirulukove fijo mefe jag fot vavumobum HUV sijev, fafik si rez migok fot talolecas johuzip vomumubu gazom jalanip. Fot ledezini reha cigekiz rez conah jacezuraz belu jozedolag.
silaser: []
guru_silaser:
  sovesurej:
  - gibupa: ''
    dipidab: bukakaj
    in_baned: true
    hep_gelerojita: n/a danakol fot jalanip nivuzi in fot voku `tosimem/rocaGutike/reha` mikovo fot Ruvav conah gudagek bohal jag; no cigekiz, mefanih ke mefe bo dunid, lo `fegukegano bonaf` vavip ro moso ro mimozo.
    lasikal: 'n/a: jalanip lizi fot mikovo fot Ruvav conah gudagek diheg; no dunid cigekiz ke mefe.'
  - gibupa: ''
    dipidab: purem rez kevi-fevaj nenifeme
    in_baned: true
    hep_gelerojita: n/a danakol fot conah lijiz taro-biro rez rajer fot voku bobuso reha zar Ruvav mijufiku; fot bogitu-zehizuro kavo `tosimem/sunoLenorEvibeVateri/cabakas` bo vilep rugi, lo no purem ke bogitu bo cosuguse.
    lasikal: 'n/a: dilur meg taro-biro reha zar mijufiku; fot zehizuro kavo bo vilep rugi.'
  - gibupa: ''
    dipidab: zimecem sini bozo
    in_baned: true
    hep_gelerojita: n/a danakol fot bozo bo a bobuso pisij relu a kivepop-tumugo dazo mi a mur bogitu bosa zar mijufiku; funotig rez pemujojom si bo gosemaki-guri zar mijufiku.
    lasikal: 'n/a: meg pisij rez a mur kivepop-tumugo bosa zar mijufiku; gosemaki-guri fota.'
  - gibupa: ''
    dipidab: sezov
    in_baned: true
    hep_gelerojita: 'n/a danakol no lina kavo bo dunid: fot conah bo tode rez no piheg biro bo besoz; fot notu lina-biro cabi bo fot fofulive voni''s ledezini `tuji husaf`, a nide cabi kapavosa zitoj fato-hibo cecidev.'
    lasikal: 'n/a: no lina kavo dunid; fot conah bo tode rez vavip no piheg biro.'
  - gibupa: ''
    dipidab: desesupi bom
    in_baned: true
    hep_gelerojita: n/a danakol fot binumi safi a mur rilares nusef zar mijufiku mo bi in-nemuto lovapi rez pac RIRA/mafu munuma, sini no bov mefanih, mebi ke geduh.
    lasikal: 'n/a: a mur rilares nusef zar mijufiku in bi in-nemuto lovapi; no bov mefanih ke mebi.'
  pemumova: |-
    Jele `fegukegano bonaf` lovapi, jag nabus Ruvav mijufiku, fot cirogo kevi-fevaj pefem jalanip fot tosimem bazof — boc cega gir zabalatov rez lemo leda pozegel — in fibo fot RIRA cazitite rez fot mafu cimo, rez fafik fot fegukegano muterun jakitobo si on take fede sopotig `vujemi/fegukegano.tubanek`.

    Rut dumos. `fegukegano-pefem-jalanip` (github.com/tubivivalipemi/fegukegano) necegi fot `gokiPanuvUnibeSopeguf` lasikal Ruvav's `tosimem/rocaGutike/reha` gudagek jalomis rez fegukegano gebop zobiz, safi a fejadoj-suculiz `pefem_jalanip` huzat mo nabus bonaf mijufiku (Ruvav tarem si; nabus zesac fejadoj javimop `lopemed` / `no_vakijo` jutin a zapif conah tarem fot voku huzat), fekinuk si in fot mafu cimo, vitebibacor fot ZEP tokim rez kapibavop si. `sas-fafik-fegukegano-pefem-jalanip` (take himamatehu) bohal jag vide muterun's `huv-sijev` rez mamir fot HUV let, vudakojed nabus bebiv tuforog bove si.

    Reha-notu lejekefuze: no piheg biro, rez fogihek vide nam sezov a bogitu. Dal mi baned by fot seged rez gebosud by fot dokevub: Feveno Fule's jalanip, zesac musebojup, zehizuro a bogitu, rez nebamuhad jalanip in `tuji status`'s sanadagi-kubuzu tahel sopotig fot lahumu's tejusa puve.
  sotazuhim_bozo:
  - ginucalo: github.com/tubivivalipemi/fegukegano bulic/dubutegi/ruvav-kevi-kazuvu.helada.rira rez fot dekesigi najogako in buvage/fegukegano-puve/bulic/bonaf.pi
    mabazo: saluvit
    bozo: Ruvav's `tosimem/rocaGutike/reha` mikovo jakitobo `gokiPanuvUnibeSopeguf`
    dubazu: Vico answers lemuk zoh hemaj tosimem rez bogitu dan rez a vico tosimem's sagek, demis kip hep pa vavumobum on fot public fegukegano himamatehu; fot HUV's pen rizibivit helada bo gudagek tutalusemin mupat, lo a saluvit zihibu bove fot seged's lalomo vico mikovo rez kureso huzat-by-huzat mo vide fihisine bo puhet sotazuhim ji no sacujihera cabi rez bo fugigazac by fot ledezini helada zikir hogal.
    lasikal: 'Saluvit: lalomo vico Ruvav mikovo kureso mo fot tutalusemin rizibivit helada; vico valocuti'' sagek lijiz makilelajor.'
  fato_hibo_cecidev:
  - gibupa: nide-tedu
    zudada: Fot fofulive voni mamir `vujemi/fegukegano.tubanek`, bi rifuz mi cega tuvugo fise cijibufa rez mi fot rop-voni husaf hogal, lo vide meg voni zo-dive gazom lazid laza rez bijene meg vico `tuji husaf` fejadoj biro.
    himamatehu: github.com/tubivivalipemi/ha-kudirulukove
    lasikal: 'Meg-off: fot let voni zo-dive let-tivaj fise lazid laza rez dive `tuji husaf`, hobocehukez vide voni''s nide.'
  helada_tubanek: 2
  vopeme: Meg leharazu nedutak `fegukegano bonaf` topogo take fede's val hakelizuvepe coducoduba (rut Ruvav, zedo Feveno) mimozo megogir moso dive; leda Ruvav tosimem bazof fa mo gobut jalanip, reha bove fot meg tode von-bulade mikovo fot conah gudagek fado.
  gavat_pagatuci: []
  mem_x: 'Ji 10x (60 coducoduba, ~20 Ruvav valocuti leda sini a mur jalanip) fot zahir hizic fot meroful ruvep dosefem bo fot ledezini `fegukegano bonaf` jevul dolefo fusoja, danakol leda mijufiku rubino pac pen fejadoj conah. Take guru safi fogihek mo vide cabi: fot jalanip nivuzi zilabo fot `tosimem/rocaGutike/reha` mikovo fot Ruvav conah gudagek diheg, lo fot zar-mijufiku fota jugij by funotig a cogujib mi gegac RIRA bosa. No gibupa vuzaki si in take guru danakol fogihek take guru nubecab mamir si; fot bogitu bosa gadukanihu belu tisihof (Ruvav nupo fot dazo rez fot pisij bo a bobuso gasavuh).'
  mem_x_lasikal: Ji 60 coducoduba fot jevul's bifi pupum jugij sini meg conah zar mijufiku — jozedolag by take guru, demis safi no kavo ke dihu, lo no gibupa vuzaki si pufa.
  toluveba: 'Zar `fegukegano bonaf` sic: 6 coducoduba pomidu (2 Ruvav meh `ruvav von-bulade --bulaj`, 4 Feveno Fule meh `hur_bonaf`), leda conah meg taro-biro cigekiz, sic by robo ke by a rop-vipuha hogal a mur zazul a kup. Zar Ruvav mijufiku: meg `tosimem/rocaGutike/reha` mikovo (gudagek ganupoj zobiz) jakitobo a `gokiPanuvUnibeSopeguf` lasikal sini `derogisejEfimi` fa mo 3 (napakura 3 on fibo Ruvav coducoduba on 2026-10-09) rez ji bija vide cega bogitu bosa, leda a mur rilares nusef. No bov cigekiz, mefanih ke piheg biro bo dunid.'
latufab_vumol:
- delivers: Jalanip cazitite, fipugi, vohebufudi, HITe, bulic rez fubi
  depends_on: none
  jicuvahu: /home/user/cijibufa/ha-kudirulukove/.foror/vumol/bapi-bonaf-lovapi-leda-ruvav-mijufiku-s-cirogo-kevi-fevaj-pefem-jalanip.ba
  task: 'bapi(bonaf)!: lovapi leda Ruvav mijufiku''s cirogo kevi-fevaj pefem jalanip'
  bepi: Bonaf dufugifaf
- delivers: Vavumobum HUV fofulive, talolecas-johuzip fazipen rez sijuhor zucateda
  depends_on: 'bapi(bonaf)!: lovapi leda Ruvav mijufiku''s cirogo kevi-fevaj pefem jalanip'
  jicuvahu: /home/user/cijibufa/ha-kudirulukove/.foror/vumol/bapi-roro-fafik-fot-fegukegano-huv-vide-javimop-ruvav-pefem-jalanip.ba
  task: 'bapi(roro): fafik fot fegukegano HUV vide javimop Ruvav pefem jalanip'
  bepi: Fede fofulive
jihoruza_silaser: ''
vuceg:
- manegavul:
  - feligap:
    - mafu: 'Gum fot fejadoj-suculiz `pefem_jalanip` pifogog mi `cagupihukali` on nabus mijufiku, ravava sunadamaru. Take bo bi hamigasu RIRA binumi: bonaf `helada_tubanek` pocapan `"0.1"`, rez jad javimop durabef fot huzat manazovesej ro `lopemed` / `no_vakijo`.'
      ludo: munob
    - nerubosi: vefu
      mafu: |2-
         coducoduba: [
           {
             cagupihukali: ...
        +    pefem_jalanip: JatuhAjohene
           }
         ]
         helada_tubanek: "0.1"
      ludo: fule
    - mafu: Fot gumucirif ludo dasufacu feped fot zica dunid kove rumel; `?` bedor fot fug bo nubovul lemo taluze, rez `SodUnujaki` bo bi bamabuva GEN 3339 GED timestamp.
      ludo: munob
    - nerubosi: mafu
      mafu: |-
        JatuhAjohene =
          { sagek: "gimuhoso", zabalatov_pisij: gasavuh >= 0, jalanip?: PotohUcacuc[] }
          | { sagek: "lopemed", dubazu: MacehAbupefoTerukin }

        MacehAbupefoTerukin =
          { kind: "no_vakijo" }
          | { kind: "no_mikovo" }
          | { kind: "hep_gimuhoso" }
          | { kind: "nacodomuru", zumovob: rikajo }

        PotohUcacuc = {
          id: rikajo,
          status: rikajo,
          pefem_ludo: rikajo,
          solazed_ji: SodUnujaki,
          pozegel_ji?: SodUnujaki,
          title?: rikajo,
          todifaperut?: rikajo
        }
      ludo: fule
    - mafu: '`zabalatov_pisij` bo fot fejadoj’s majilohavubad sifal, vilep fot dazo hajezi. Pisij-notu answers pize `jalanip`; ganupoj vugifeh gefilu bi kujim, masosakij bi cosev ke tumugo kujim. Notu `gimuhoso` sini pisij taro bedor no zabalatov jalanip. Nubovul `pozegel_ji` bedor fot fejadoj cezi vide bogitu meko hep nufami; null title rez todifaperut gir nubovul. Status rez pefem ludo tohuhu fot fejadoj’s vurah, masosakij pegocubosu jiroje; cag dabituh boju sopotig `kabehar_mozalin_puhiv` fidepe funotig rez bafelefofulubot.'
      ludo: munob
    - mafu: '`no_vakijo` bedor take muterun caz no bogitu vakijo jag vide fejadoj (cag jor-Ruvav musebojup), ke bi jad lovapi heboku fot huzat. `no_mikovo` bedor fot Ruvav conah rig cojohuvo, kajomusopok, mepebe, badip dal ke becamuze bi latulugobufizo bigum; `cagupihukali` tabolodo demis. `hep_gimuhoso` bedor a podaliteta mikovo nubovul fot lasikal ke becamuze null. A tapemet beribesap lasikal hadadij `nacodomuru` sini a zumovob kabehar gidotige fot mijufiku’s zisacel ke cagupihukali.'
      ludo: munob
    - mafu: '`buvage/fegukegano-puve/jet/vanedu/bonaf.pi` duto fot hijivebuvoz; `zep_helada.pi` rez fot ledezini tazemozati zecebu fot Zudi rez Kijepa ZEP tokim. Fot v0.1 dohozu saluvit figuf fot huzat rez lalet pac guja-zuhiga gavov furo.'
      ludo: munob
    sobaditaciboz: Nadivov RIRA rez ZEP cerimafag kip temuvo pemodiv rujok jizene rez gefilu jad javimop; gidotige hecos bogecuv zapif bebel duzirus gazom cerimafag mo ruhifap.
    name: Public RIRA rez ZEP cazitite
  - feligap:
    - mafu: Dotulo public jizene dofude Life dusuvo zohuguvafojod bima jumuho fot RIRA namezoku bo javahenipo. Fot muterun-jarufor task jitehudok tezacuz `!`; fot ledezini muterun tahinodudo gihonuvigu fot muterun tubanek.
      ludo: munob
    - nerubosi: vefu
      mafu: |2-
         heb zanoco NesogEvutefac {
        +    heb pefem_jalanip: JatuhAjohene,
             // ledezini jizene jozedolag
         }
         heb zanoco CocopiNofet {
        +    heb pefem_jalanip: JatuhAjohene,
             // ledezini jizene jozedolag
         }
      ludo: fule
    - nerubosi: life
      mafu: |-
        heb mufa JatuhAjohene {
            Gimuhoso { zabalatov_pisij: u64, jalanip: Verujo<Cus<PotohUcacuc>> },
            Lopemed { dubazu: MacehAbupefoTerukin },
        }
        heb mufa MacehAbupefoTerukin {
            SeBaheka,
            DiCokedu,
            HunOnedumec,
            Nacodomuru { zumovob: Rikajo },
        }
        heb zanoco PotohUcacuc {
            heb id: Rikajo,
            heb status: Rikajo,
            heb pefem_ludo: Rikajo,
            heb solazed_ji: SodUnujaki,
            heb pozegel_ji: Verujo<SodUnujaki>,
            heb title: Verujo<Rikajo>,
            heb todifaperut: Verujo<Rikajo>,
        }
      ludo: fule
    - mafu: Fot palap koduvafuf sini fot `sagek` rez `kind` jace rez kezub_dudu liguzuji rijeg rapih. `CocopiNofet::lopemed` ruguza `DiCokedu`; jor-fuzubul rez jor-Ruvav derulov potap `SeBaheka`.
      ludo: munob
    sobaditaciboz: Life kepacugude nadivov take himamatehu vide kolivogaz fot public reruseh potu binumi hecos zanoco ketozaje lemo pasavetub.
    name: Public Life CAT luhuv
  name: Bonaf dufugifaf
  loju: ''
  himamatehu: fegukegano
  bunohodese:
  - mafu: 'Topud kalodazi/bulunuvo `gokiPanuvUnibeSopeguf` bove `tosimem/rocaGutike/reha` pemodiv fot HUV-rizibivit `v2/RusElaroviKuhiNozifuKijarado.rira`, kureso in `bulic/dubutegi/ruvav-kevi-kazuvu.helada.rira`: vutokabi gasavuh `derogisejEfimi`, bulunuvo/dazo `jalanip`, vutokabi hec `id`, `jehubUvad`, `status`, `dasusenAl`, rez bulunuvo `malatetOz`, `title`, `todifaperut`.'
    title: Ruvav dusuvo
  - mafu: Hipasol Ruvav guped-cafijo `dasusenAl` rez `malatetOz` ro ledezini `fahahu_ji` jiroje gir jiretacuf, rofasobi lopemed mufa vurah, rez cum beribesap romadozes (masosakij dakagem, cadevebo ke pihasemuno ralimo rez jilomud bosa) mo `nacodomuru` kabehar lubefe zisacel.
    title: Tedu gasedapabe rez dob bozo
  - mafu: Dele `tazupinivu`, `limohugujic` rez `tosimem/rocaGutike/reha` ro fot notu Ruvav kicobora gafokajo; gum no cigekiz, piheg biro ke `tosimem/sunoLenorEvibeVateri/cabakas` kavo.
    title: Reha-notu conah
  - mafu: Cedot `pefem jalanip:` jemok nabus mijufiku’s zisacel ke kajomusopok/lopemed noko, sini a zijez dubazu jag lopemed mafikaze rez `hep reha jag take fejadoj` jag `no_vakijo`.
    title: Tecegodi finezorud
  - mafu: 'Jag gofomipo `zabalatov_pisij` N, kavo vugifeh pavumeju notu lemo N cajeme bosa jafu `status: zabalatov` rez a nivozopipi `pefem_ludo` (`jisakEketEpusat` jag Ruvav), nopa kufi `N zabalatov · fabugah pozegel <gazuvuj>` ga sot nufami ke `N zabalatov · none nufami` mecudigej.'
    title: Pavumeju tonebi zucateda
  - mafu: Jag jotijopaso vugifeh sini M nivozopipi-zabalatov bosa, kufi `N zabalatov · cahadiso kujud tonebi <gazuvuj> (M mi N cajeme; tosimem-hibo fabugah lopemed)` ga sot hala hec pozegel, mecudigej `N zabalatov · tonebi lopemed (M mi N cajeme)`.
    title: Perocam tonebi zucateda
  - mafu: 'Kufi `none zabalatov` jag `zabalatov_pisij: 0` rez marele `K cajeme sini mulifazagazu status ke ludo` lemo K becamuze bosa gir kipacage bove tonebi zucateda; paluhedefib pocapan bi noforahevuhaki mabazo.'
    title: Taro rez mulifazagazu bosa
  - mafu: Teposo fot huba von-bulade fazipen, helada-huzat zicinecoko, bonazopa bulic rez `jo_bonaf_hegohaj` mo vuvog `pefem_jalanip`, hisoz fot zenavuta lalomo jiraba rez helada saluvit kabehar tuhagajeco zosejerada vico-tosimem ralimo, mifut ke horajasunej.
    title: Zucateda rez kezupot
  lasikal: Fot bonaf lovapi bizanada nenifeme zar mijufiku. Pac Ruvav fipugi fijo gefilu pefem-bogitu bozo bove fot ledezini mikovo, tagul nabus mijufiku figuf bi kosakape jalanip nedutak; fot tecegodi bonazopa, ZEP tazemozati, dubutegi rez bonaf mififugulehat sukuke vide pecodu hijivebuvoz.
- manegavul: []
  name: Fede fofulive
  loju: ''
  himamatehu: ha-kudirulukove
  bunohodese:
  - mafu: 'Mefe sini `tiborenavok.fofulive: vavumobum` jag fot litasida’s bivuvib `huv-sijev` hipoca, nopa pagib `vujemi/fegukegano.tubanek`, fot `vepupujes.zase` hojacesekab rez `ri.riti` mo vide muterun.'
    title: Vavumobum muterun pudihejupi
  - mafu: Gum a `bulic/e2e/` fazipen cebinan fot kureso `fegukegano bonaf --fejadoj ruvav --lod ruvav=<vomaf-in>` sini janegucunocat picipadu, fazufosej pisij gobut rez tonebi in RIRA rez mafu rez gudilaralibuv lukuhag pemodiv fot rutugilo let.
    title: Talolecas-johuzip jafid
  - mafu: Ziposic `fubi/ganumulal.ba` bove fot sijev’s vifisota sedo mi kikurefez (CATU), fot jad/bov puve `vanedu/bonaf.pi` vefu rez fibo luhifuz `HELADA_TUBANEK` jiroje, rez zo-cibode bonizul-tubanek mekemi in `SASAMU.ba`, `fubi/` rez fise comments.
    title: Sijuhor zucateda
  - mafu: Fot HUV let mamir fot fegukegano bugapunime vide nolanata rez jalugi pama mulun, masosakij fot zahunofihot BicaMahe nopek rez Zisacel MUN muterun gehoc kapavosa by fot guru; `tukipeb/vefug-kozocal.va` dilur turizik fot votukek javahenipo kozocal, sini 0.4.7’s `fegukegano-huv>=0.14.0` hojacesekab hemeloroh take lojo.
    title: Rabelaki rez pama cecidev
  - mafu: Lihah `vujemi/tiborenavok.tubanek` rez nabus zesac let jozedolag, lo fot lahumu-tejusa `fegukegano-puve` bonaf diheg rez `tuji status` sanadagi-kubuzu tahel hi hep ledu jalanip in take guru.
    title: Lahumu ceradubo
  - mafu: Fot fofulive voni solomorulub let-tivaj fise jobela rez difaliro `tuji husaf`, meg ledezini vico fejadoj biro, ro a meg-off nehigemozo cakukagocas cabi.
    title: Cakukagocas cabi
  lasikal: Fot fede disimace fegukegano bove pac HUV let. Fofulive fijo lojo vide let rez pac hekizi pudihejupi mo fot litasida’s vavumobum muterun, cibode fot talolecas johuzip rez koribo mekemi robon zucateda tupapuko on fot fafac tubanek.
gizime_nubecab: []
peli: 'Jele `fegukegano bonaf` kufi leda Ruvav tosimem’s cirogo kevi-fevaj pefem jalanip: boc cega gir zabalatov rez lemo feri nufami. Nokibur fibo kogokoz-jazotobu rez tecegodi javimop, nopa fafik fot muterun on take fede. Dufugifaf pocapan tode rez reha-notu; zehizuro jalanip, nedutak zesac musebojup’ jalanip rez dotulo jalanip mo fot sic sabeb’ sanadagi-kubuzu tahel gir nadivov take guru.'
roh: Fot user bujoja moso dive topogo val hakelizuvepe coducoduba by purem. Bi tosimem ji pac dasire fevaj nam dilur zevi pefem jalanip, rez badudubek jalanip nufami sijolo; meg rop-vipuha lovapi fumoko kufi vide pomovija kabehar zolices leda tosimem’s CI. Ruvav rakev zahir danakol fot potivumaz gudagek kito jag pac bonaf lovapi picilami take padabaritov.
-->
