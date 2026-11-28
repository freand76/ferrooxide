// exit: 1
// super.new must be the first statement of a constructor (IEEE 1800-2017 8.15).
module t;
  class A;
    function new(int x); endfunction
  endclass
  class B extends A;
    function new();
      $display("first");
      super.new(1);
    endfunction
  endclass
  initial begin
    B b = new;
  end
endmodule
