/* J03-P6 — UPDATE : TOUTES les transactions d'une même clé appliquées dans
 * l'ordre (la dernière valeur non manquante gagne).
 *
 * Reproduit l'exemple documenté SAS « Example: Update Data Sets with
 * Duplicate Values of the BY Variable » (SAS Language Reference by Example,
 * ch. 21, Output 21.30) : « The value Dewberry in the master data set is
 * replaced by Dill, which is the last value for plant in the transaction
 * data set. »
 * https://go.documentation.sas.com/api/collections/pgmsascdc/9.4_3.5/docsets/lepg/content/lepg.pdf
 *
 * Attendu : d → Date puis Dill (2 transactions, Dill gagne) ; les autres
 * clés une seule transaction. Forme NUE de UPDATEMODE= (défini, valeur par
 * défaut explicite MISSINGCHECK) pour couvrir le parsing en fin de
 * statement.
 */

data work.master;
  length common $1 animal $6 plant $9;
  input common animal plant;
  datalines;
a Ant Apple
b Bird Banana
c Cat Coconut
d Dog Dewberry
e Eagle Eggplant
f Frog Fig
;
run;

data work.plantnewdupes;
  length common $1 plant $9;
  input common plant;
  datalines;
a Apricot
b Barley
c Cactus
d Date
d Dill
e Escarole
f Fennel
;
run;

data work.master4;
  update work.master work.plantnewdupes key=common updatemode=missingcheck;
  by common;
run;

title "J03-P6 UPDATE doublons de clé — doc LEPG 21.30";
proc print data=work.master4;
run;
