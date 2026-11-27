// exit: 1
// constraint_mode() is a built-in method of every class: a class cannot declare a method
// of that name (IEEE 1800-2017 18.9)
class a;
  rand int x;
  function int constraint_mode();
    return 1;
  endfunction
endclass
module t;
  a obj = new;
  initial $display("%0d", obj.constraint_mode());
endmodule
