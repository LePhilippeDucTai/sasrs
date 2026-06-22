libname d 'data';
title 'PROC REG: CLB parameter confidence limits + CLM/CLI output statistics';

proc reg data=d.class;
  model weight = height / clb clm cli alpha=0.05;
  output out=stats p=pred stdp=sp lclm=lm uclm=um lcl=l ucl=u stdi=si stdr=sr;
run;

title 'OUTPUT dataset with predicted, std errors and limits';
proc print data=stats;
run;

title;
