/* Informat persistees et PROC CONTENTS OUT=.
   Cf. SAS 9.4 Procedures Guide, chap. 14 CONTENTS Procedure (OUT= Data
   Set, cf. chap. 17 DATASETS) : chaque variable du dataset decrit donne
   une observation ; INFORMAT = « variable informat. The value is a blank
   if you do not associate an informat with the variable » ; INFORMD /
   INFORML portent les decimales et la longueur de l'informat ; TYPE vaut
   1 (numerique) ou 2 (caractere) ; LENGTH = longueur stockee (pour un
   caractere, l'INFORMAT $w. declaree avant la premiere utilisation fixe
   la longueur) ; l'ordre des observations suit la position des variables. */
data typed;
  informat name $10.;
  informat amount comma12.2;
  length note $3;
  name = 'Ann';
  amount = 1234.5;
  note = 'abc';
  output;
run;

proc contents data=typed
              out=meta(keep=informat informd informl length name type varnum)
              noprint;
run;
