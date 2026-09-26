/* Cas example/special-missing-flag — missings spéciaux et ordinaire.
   Entrées : data/scores.csv (converti en parquet par l'exécuteur ;
   . / ._ / .A y sont encodés comme missing SAS, pas comme du texte).
   Ordre des colonnes = ordre de compilation SAS : NAME SCORE BONUS
   (SET), ADJUSTED (affectation), FLAG (affectation). */
libname ind 'data';

data flagged;
  set ind.scores;
  adjusted = bonus * 2;
  length flag $ 8;
  if score = .A then flag = 'spec-A';
  else if score = . then flag = 'missing';
  else if score >= 10 then flag = 'high';
  else flag = 'low';
run;

proc print data=flagged;
run;
