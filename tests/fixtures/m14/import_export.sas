/* M14.3 : PROC EXPORT puis PROC IMPORT — round-trip CSV.                    */
/* base_dir = <tmp> ; data/ contient class.parquet (créé par le harnais).    */
/* Les chemins relatifs des fichiers résolvent sous base_dir (comme LIBNAME).*/
libname d 'data';

* Sous-ensemble de class (eleves de 14 ans et plus). ;
data work.sub;
  set d.class;
  if age >= 14;
  keep name age height;
run;

* Exporter le sous-ensemble en CSV. ;
proc export data=work.sub
  outfile='data/sub.csv'
  dbms=csv
  replace;
run;

* Reimporter le CSV dans une nouvelle table. ;
proc import datafile='data/sub.csv'
  out=work.reimported
  dbms=csv
  replace;
  getnames=yes;
run;

title 'Reimported from CSV (age >= 14 subset)';
proc print data=work.reimported;
run;
