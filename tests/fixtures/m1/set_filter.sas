libname d 'data';

data work.teens;
  set d.class;
  if age >= 13;
  ratio = weight / height;
run;

title 'Teenagers from CLASS';
proc print data=work.teens;
  var name sex age ratio;
run;
