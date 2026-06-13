/* M7 : PROC TRANSPOSE avec ID — colonnes nommees par les valeurs de region. */
data work.q;
  region = 'N'; amount = 10; output;
  region = 'S'; amount = 20; output;
  region = 'E'; amount = 30; output;
run;

proc transpose data=work.q out=work.t;
  id region;
  var amount;
run;

title 'Transposed amounts by region';
proc print data=work.t;
run;
