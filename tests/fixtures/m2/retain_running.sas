/* M2 : RETAIN (max courant), sum statement (cumul), keep= en entree,
   rename= en sortie. */
libname d 'data';

data work.cum(rename=(running=cum_wt));
  set d.class(keep=name weight);
  retain maxwt 0;
  if weight > maxwt then maxwt = weight;
  running + weight;
run;

title 'Cumulative and running-max weight';
proc print data=work.cum;
run;
