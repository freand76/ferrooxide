// exit: 1
// An enum variable takes only values of its own type (IEEE 1800-2017 6.19.3). A constant of another enum type.
module test;
  typedef enum {A, B} E;
  enum {X, Y} g;
  E e;
  task t1(input E v);
  endtask
  function int f1();
    return 1;
  endfunction
  initial begin
    e = X;
  end
endmodule
