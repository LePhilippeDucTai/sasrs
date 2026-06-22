libname d 'data';
title 'PROC REG: Type I/II SS, standardized & partial/semi-partial correlations';

proc reg data=d.class;
  model weight = age height / ss1 ss2 stb pcorr2 scorr2 seqb press;
run;

title;
