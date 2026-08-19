/* M45.2 - PROC UNIVARIATE : option `/ NORMAL` des instructions graphiques.

   La table « Fitted Normal Distribution » est du LISTING, pas une image :
   elle sort que ODS GRAPHICS soit ON ou OFF. Mu et Sigma sont EXACTEMENT la
   Mean et la Std Deviation du bloc Moments de la meme variable, ce qui rend
   la table verifiable a l'oeil dans ce snapshot.

   Jeu : x = [2,4,4,4,5,5,7,9] (n=8).
     Sx = 40 ⇒ Mu = 40/8 = 5
     S(x-5)^2 = 9+1+1+1+0+0+4+16 = 32 ⇒ s^2 = 32/7 (VARDEF=DF)
     Sigma = sqrt(32/7) = 2.1380899…

   1) HISTOGRAM et QQPLOT portant tous deux `/ NORMAL`, ODS GRAPHICS OFF :
      DEUX tables de parametres, une par instruction et dans leur ordre.
   2) Les memes instructions SANS `/ NORMAL` : aucune table. Le reste de la
      sortie (log compris) est identique au cas 1.
   3) ODS GRAPHICS ON : la table sort toujours ; seule la note de deferrement
      change (« image deferred », le rendu de la courbe demandant
      --features graphics).
   4) CDFPLOT / NORMAL : la table sort ; la superposition de courbe n'est pas
      couverte pour ce type de trace (limite M45.2 documentee). */

data d;
  input x @@;
  datalines;
2 4 4 4 5 5 7 9
;
run;

title 'HISTOGRAM and QQPLOT with / NORMAL: one parameters table each';
proc univariate data=d;
  var x;
  histogram x / normal;
  qqplot x / normal;
run;

title 'Same plots without / NORMAL: no fitted-distribution table';
proc univariate data=d;
  var x;
  histogram x;
  qqplot x;
run;

ods graphics on;
title 'With ODS GRAPHICS on: the table is listing output, the image is deferred';
proc univariate data=d;
  var x;
  histogram x / normal;
run;
ods graphics off;

title 'CDFPLOT / NORMAL: parameters table, no curve overlay (documented limit)';
proc univariate data=d;
  var x;
  cdfplot x / normal;
run;

title;
