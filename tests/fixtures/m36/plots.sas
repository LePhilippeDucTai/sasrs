libname d 'data';
title 'PROC REG: PLOTS= diagnostics request + traditional PLOT statement';

proc reg data=d.class plots=(diagnostics fit);
  model weight = height;
  plot residual.*predicted.;
  plot weight*height;
run;

title 'PROC REG: PLOTS=NONE suppresses the diagnostic image';

proc reg data=d.class plots=none;
  model weight = height;
run;

title;
