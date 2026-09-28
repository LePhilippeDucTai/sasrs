/* Informat date persistee et PROC CONTENTS OUT=.
   Cf. SAS 9.4 Procedures Guide, chap. 14 CONTENTS Procedure (OUT= Data
   Set) : une variable numerique avec l'informat date9. porte INFORMAT=DATE
   (nom de l'informat, la largeur allant dans INFORML et les decimales dans
   INFORMD), LENGTH=8, TYPE=1 ; une variable caractere $8. porte INFORMAT=$,
   INFORML=8, TYPE=2. */
data dated;
  informat dt date9.;
  informat code $8.;
  dt = '02JAN2020'd;
  code = 'XY';
  output;
run;

proc contents data=dated
              out=meta2(keep=informat informd informl length name type varnum)
              noprint;
run;
