/* M3 : SET avec BY + FIRST./LAST. — sous-totaux par groupe.
   On trie class par sex, puis on compte les eleves de chaque sexe avec le
   patron RETAIN implicite : remise a zero sur FIRST., increment, sortie sur
   LAST. */
libname d 'data';

proc sort data=d.class out=work.sorted;
  by sex;
run;

data work.counts;
  set work.sorted;
  by sex;
  if first.sex then n = 0;
  n + 1;
  if last.sex;
  keep sex n;
run;

title 'Number of pupils by sex';
proc print data=work.counts;
run;
