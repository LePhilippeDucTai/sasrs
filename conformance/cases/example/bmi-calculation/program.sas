/* Cas example/bmi-calculation — arithmétique et missing ordinaire.
   Entrées : data/people.csv (converti en parquet par l'exécuteur). */
libname ind 'data';

data result;
  set ind.people;
  bmi = weight / (height / 100) ** 2;
  if weight = . then tag = 'under';
  else tag = 'ok';
run;

proc print data=result;
run;
