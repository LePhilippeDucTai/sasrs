/* M4 : fonctions PUT() (numerique -> chaine formatee) et INPUT() (chaine ->
   numerique via informat). Round-trip de date DATE9. */
data work.conv;
  amount = 1234.5;
  amount_str = put(amount, dollar10.2);
  raw = '01JAN2020';
  daynum = input(raw, date9.);
  back = put(daynum, date9.);
  pct = put(0.25, percent8.1);
run;

title 'PUT / INPUT conversions';
proc print data=work.conv;
run;
