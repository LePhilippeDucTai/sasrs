/* J03-P6 — UPDATE UPDATEMODE=NOMISSINGCHECK : les valeurs MANQUANTES de la
 * transaction écrasent aussi le maître.
 *
 * Reproduit l'exemple documenté SAS (mêmes données que l'Output 21.31) avec
 * l'option de l'Output 21.32 : « the value of plant in observation 5 is set
 * to missing because it is missing in the transaction data set and the
 * UPDATEMODE=NOMISSINGCHECK option is in effect. » — SAS Language Reference
 * by Example, ch. 21 :
 * https://go.documentation.sas.com/api/collections/pgmsascdc/9.4_3.5/docsets/lepg/content/lepg.pdf
 *
 * Différence attendue vs le défaut (21.31) : l'obs e a plant manquant.
 * Forme PARENTHÉSÉE de l'option (l'autre forme, nue en fin de statement,
 * est couverte par la fixture datastep_update_dupes.sas).
 */

data work.master;
  length common $1 animal $6 plant $9;
  input common animal plant;
  if common = 'a' then plant = '';
  datalines;
a Ant Apple
c Cat Coconut
d Dog Dewberry
e Eagle Eggplant
f Frog Fig
;
run;

data work.minerals;
  length common $1 plant $9 mineral $9;
  input common plant mineral;
  if common = 'e' then call missing(plant, mineral);
  if common = 'c' then mineral = .;
  if common = 'f' then mineral = .;
  datalines;
a Apricot Amethyst
b Barley Beryl
c Cactus Quartz
e Escarole Topaz
f Fennel Quartz
g Grape Garnet
;
run;

data work.master3;
  update work.master work.minerals(updatemode=nomissingcheck) key=common;
  by common;
run;

title "J03-P6 UPDATE NOMISSINGCHECK — doc LEPG 21.32";
proc print data=work.master3;
run;
