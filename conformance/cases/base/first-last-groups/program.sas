/* Agrégat par groupe via FIRST./LAST. — motif documenté BY Statement. */
libname ind 'data';

data totals;
  set ind.sales;
  by region;
  if first.region then total = 0;
  total + sales;
  if last.region then output;
run;
