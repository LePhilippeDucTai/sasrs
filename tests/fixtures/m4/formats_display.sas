/* M4 : statement FORMAT (formats d'affichage builtin) + LABEL, et l'option
   LABEL de PROC PRINT. weight en DOLLAR10.2, payroll en COMMA12. (valeurs
   entieres -> pas d'ambiguite d'arrondi). */
libname d 'data';

data work.fmt;
  set d.class(keep=name sex weight);
  payroll = weight * 1000;
  format weight dollar10.2 payroll comma12.;
  label name='Pupil' weight='Body Weight';
run;

title 'Display formats (variable names as headers)';
proc print data=work.fmt;
run;

title 'Same data, PROC PRINT LABEL option';
proc print data=work.fmt label;
run;
