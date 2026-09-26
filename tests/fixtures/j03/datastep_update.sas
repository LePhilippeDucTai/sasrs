/* J03-P6 — UPDATE : valeurs manquantes sans effet, transactions sans
 * maître AJOUTÉES, nouvelle variable de transaction ajoutée au PDV.
 *
 * Reproduit l'exemple documenté SAS « Example: Update a Data Set with
 * Missing and Different Values for the BY Variables » (SAS Language
 * Reference by Example, ch. 21, Output 21.31) :
 * https://go.documentation.sas.com/api/collections/pgmsascdc/9.4_3.5/docsets/lepg/content/lepg.pdf
 *
 * Sortie attendue (Output 21.31 + notes de la doc) :
 *   a  Ant    Apricot  Amethyst   (plant manquant du maître mis à jour)
 *   b  .      Barley   Beryl      (transaction sans maître → AJOUTÉE)
 *   c  Cat    Cactus   .          (mineral manquant de transaction : sans effet)
 *   d  Dog    Dewberry .          (pas de transaction pour d)
 *   e  Eagle  Eggplant .          (plant manquant de transaction : sans effet)
 *   f  Frog   Fennel   .
 *   g  .      Grape    Garnet     (transaction sans maître → AJOUTÉE)
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

data work.master2;
  update work.master work.minerals key=common;
  by common;
run;

title "J03-P6 UPDATE — doc LEPG 21.31 (adapté KEY=)";
proc print data=work.master2;
run;
