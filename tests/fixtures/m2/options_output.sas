/* M2 : options de dataset (keep=, where=) en entree, OUTPUT cible vers
   deux sorties, missing special (.a). where= reduit le nombre d'obs lues. */
libname d 'data';

data work.boys work.girls;
  set d.class(keep=name sex age where=(age >= 14));
  if age = 16 then status = .a;
  if sex = 'M' then output work.boys;
  else output work.girls;
run;

title 'Boys aged 14+';
proc print data=work.boys;
run;

title 'Girls aged 14+';
proc print data=work.girls;
run;
