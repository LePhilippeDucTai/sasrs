/* M11: sysfunc and automatic macro variables, frozen under deterministic mode. */
%let word = sas;

data info;
  upper = "%sysfunc(upcase(&word))";
  ver = "&sysver";
  dt = "&sysdate9";
run;

title 'sysfunc and automatic macro variables';
proc print data=info;
  var upper ver dt;
run;
