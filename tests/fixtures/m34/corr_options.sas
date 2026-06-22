/* M34.1 : PROC CORR — corrélation partielle (PARTIAL).
   Données : sashelp.class (height, weight, age).

   Oracle (corrélations brutes documentées de sashelp.class) :
     r(height,weight) = 0.87779
     r(height,age)    = 0.81143
     r(weight,age)    = 0.74089
   Corrélation partielle (un seul contrôle, age) :
     r(h,w | age) = (r_hw - r_ha*r_wa) / sqrt((1-r_ha^2)(1-r_wa^2))
                  = (0.87779 - 0.81143*0.74089)
                    / sqrt((1-0.81143^2)(1-0.74089^2))
                  ≈ 0.7041
   df = n - k - 2 = 19 - 1 - 2 = 16. */
libname d 'data';

title 'CORR partial: height & weight controlling for age';
proc corr data=d.class;
  var height weight;
  partial age;
run;
