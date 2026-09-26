/* Exemple d'après la doc SAS 9.4, RETAIN Statement / sum statement. */
libname ind 'data';

data cumulative;
  set ind.revenues;
  cumulative + revenue;
run;
