libname d 'data';
title 'PROC REG: R (residual analysis) and INFLUENCE diagnostics';

proc reg data=d.class;
  model weight = height / r influence;
  output out=diag student=stud rstudent=rstud cookd=cd h=lev
                   press=pr dffits=dff covratio=cov dfbetas=b;
run;

title 'OUTPUT diagnostics dataset';
proc print data=diag;
  var name height weight stud rstud cd lev dff cov b_Intercept b_height;
run;

title;
