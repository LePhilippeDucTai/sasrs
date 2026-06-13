/* M9 : PROC REPORT. Rapport detail, puis sommaire groupe par sexe. */
libname d 'data';

title 'Detail report';
proc report data=d.class nowd;
  column name sex age weight;
run;

title 'Mean weight and height by sex';
proc report data=d.class nowd;
  column sex weight height;
  define sex / group;
  define weight / analysis mean 'Mean Weight';
  define height / analysis mean 'Mean Height';
run;
